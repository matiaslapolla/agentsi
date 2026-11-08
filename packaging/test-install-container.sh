#!/bin/sh
# Runs install.sh in a throwaway container, never on this machine.
#
#   packaging/test-install-container.sh <dir with agentsi and agentsi-core> [image]
#
# The image must have systemd-analyze (default: archlinux, the glibc of an Arch host).
# The container runs as a user without privileges. It checks that:
#   - the programs and the unit land in the user's home and run,
#   - a dry run changes nothing,
#   - --bin-dir outside the home is refused,
#   - systemd-analyze verify accepts both units, core and pill (and rejects a broken one),
#   - no file outside the home and /tmp changed.
set -eu
built=${1:?usage: test-install-container.sh <dir with agentsi and agentsi-core> [image]}
image=${2:-archlinux}
here=$(cd "$(dirname "$0")" && pwd)
built=$(cd "$built" && pwd)

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
echo "systemd-analyze verify: ok"
printf "[Service]\\nExecStart=/nonexistent/x\\nBogusKey=1\\n" > /tmp/bad.service
if systemd-analyze verify /tmp/bad.service >/dev/null 2>&1; then echo "FAIL: verify accepts a broken unit"; exit 1; fi
echo "a broken unit is rejected: ok"
echo "== files changed outside /home/tester and /tmp"
changed=$(find / -xdev \( -path /home/tester -o -path /tmp -o -path /proc -o -path /sys -o -path /run -o -path /dev \) -prune -o -type f -newer /marker -print | grep -v "^/marker$" || true)
echo "${changed:-none}"
[ -z "$changed" ]
'
