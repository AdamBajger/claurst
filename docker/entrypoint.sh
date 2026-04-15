#!/usr/bin/env bash
# entrypoint.sh — container startup for the claurst SSH host.
set -euo pipefail

# ── SSH host keys ─────────────────────────────────────────────────────────────
# Generate host keys on first start (or when the container has no persistent
# storage for /etc/ssh).  ssh-keygen -A regenerates all key types that are
# referenced in sshd_config but currently missing.
if [[ ! -f /etc/ssh/ssh_host_ed25519_key ]]; then
    echo "[entrypoint] Generating SSH host keys..."
    ssh-keygen -A
fi

# ── Authorized public keys ────────────────────────────────────────────────────
# Populate /home/claurst/.ssh/authorized_keys from the AUTHORIZED_KEYS
# environment variable so that the container is usable without a volume mount.
#
# Usage:
#   docker run -e AUTHORIZED_KEYS="$(cat ~/.ssh/id_ed25519.pub)" ghcr.io/<repo>
#
# For multiple keys, separate them with newlines inside the variable.
if [[ -n "${AUTHORIZED_KEYS:-}" ]]; then
    echo "${AUTHORIZED_KEYS}" > /home/claurst/.ssh/authorized_keys
    chmod 600 /home/claurst/.ssh/authorized_keys
    chown claurst:claurst /home/claurst/.ssh/authorized_keys
    echo "[entrypoint] Installed $(wc -l < /home/claurst/.ssh/authorized_keys) authorized key(s)."
else
    echo "[entrypoint] WARNING: AUTHORIZED_KEYS is not set." >&2
    echo "[entrypoint]          No SSH public keys installed; logins will be rejected." >&2
    echo "[entrypoint]          Pass -e AUTHORIZED_KEYS=\"\$(cat ~/.ssh/id_ed25519.pub)\" to docker run." >&2
fi

# ── Start SSH daemon ──────────────────────────────────────────────────────────
echo "[entrypoint] Starting sshd..."
exec /usr/sbin/sshd -D -e
