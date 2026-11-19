#!/bin/sh
# Runs install.sh in a throwaway container, never on this machine.
#
#   packaging/test-install-container.sh <dir with agentsi and agentsi-core> [base image]
#
# It builds packaging/test-image/Dockerfile on the base image, which adds systemd-analyze and
# desktop-file-validate. Default base: archlinux (the glibc of an Arch host) on x86_64, and
# ubuntu:24.04 on aarch64, where archlinux has no image. On ubuntu:24.04, pass the programs of
# make dist-portable (glibc 2.39).
# The container runs as a user without privileges, with no network. It checks that:
#   - the programs, the units, the desktop entry and the app icon land in the user's home and run,
#   - a dry run changes nothing,
#   - a bad path is refused before any change,
#   - the restart of the services, with a fake systemctl,
#   - systemd-analyze verify accepts both units, core and pill (and rejects a broken one),
#   - no file outside the home and /tmp changed.
set -eu
built=${1:?usage: test-install-container.sh <dir with agentsi and agentsi-core> [base image]}
case "$(uname -m)" in aarch64|arm64) base=${2:-ubuntu:24.04} ;; *) base=${2:-archlinux} ;; esac
here=$(cd "$(dirname "$0")" && pwd)
built=$(cd "$built" && pwd)
image="agentsi-installtest:$(echo "$base" | tr -c 'a-zA-Z0-9.\n' '-')"
docker build -q --build-arg BASE="$base" -t "$image" "$here/test-image" >/dev/null

exec docker run --rm --network none \
  -v "$built":/in:ro -v "$here":/pkg:ro "$image" sh -eu -c '
useradd -m tester
touch /marker
sleep 1
cp -r /pkg /tmp/pkg
chown -R tester /tmp/pkg
su tester -c "sh /tmp/pkg/test-inside.sh /tmp/pkg"
echo "== the units are valid (systemd-analyze verify, as root: it cannot start a user manager here)"
systemd-analyze verify /home/tester/.config/systemd/user/agentsi-core.service
systemd-analyze verify /home/tester/.config/systemd/user/agentsi-pill.service
for u in /tmp/space-units/*.service; do systemd-analyze verify "$u"; done
echo "systemd-analyze verify: ok (also with a space in the bin dir)"
printf "[Service]\\nExecStart=/nonexistent/x\\nBogusKey=1\\n" > /tmp/bad.service
if systemd-analyze verify /tmp/bad.service >/dev/null 2>&1; then echo "FAIL: verify accepts a broken unit"; exit 1; fi
echo "a broken unit is rejected: ok"
echo "== files changed outside /home/tester and /tmp"
changed=$(find / -xdev \( -path /home/tester -o -path /tmp -o -path /proc -o -path /sys -o -path /run -o -path /dev \) -prune -o -type f -newer /marker -print | grep -v "^/marker$" || true)
echo "${changed:-none}"
[ -z "$changed" ]
'
