# The image of packaging/test-install-container.sh: a base image plus systemd-analyze and
# desktop-file-validate. The base is a build argument, because archlinux has no aarch64 image.
ARG BASE=archlinux
FROM ${BASE}
RUN if command -v pacman >/dev/null 2>&1; then \
      pacman -Syu --noconfirm --needed systemd desktop-file-utils && pacman -Scc --noconfirm; \
    else \
      apt-get update && apt-get install -y --no-install-recommends systemd desktop-file-utils \
        && rm -rf /var/lib/apt/lists/*; \
    fi
