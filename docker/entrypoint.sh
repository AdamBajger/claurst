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
    _tmp_line=$(mktemp)
    _tmp_keys=$(mktemp)
    _valid=0
    _invalid=0
    while IFS= read -r _line; do
        # Skip blank lines and comments.
        [[ -z "${_line}" || "${_line}" =~ ^[[:space:]]*# ]] && continue
        printf '%s\n' "${_line}" > "${_tmp_line}"
        if ssh-keygen -l -f "${_tmp_line}" > /dev/null 2>&1; then
            printf '%s\n' "${_line}" >> "${_tmp_keys}"
            _valid=$(( _valid + 1 ))
        else
            echo "[entrypoint] WARNING: Skipping invalid SSH key entry." >&2
            _invalid=$(( _invalid + 1 ))
        fi
    done <<< "${AUTHORIZED_KEYS}"
    rm -f "${_tmp_line}"
    if [[ "${_valid}" -gt 0 ]]; then
        mv "${_tmp_keys}" /home/claurst/.ssh/authorized_keys
        chmod 600 /home/claurst/.ssh/authorized_keys
        chown claurst:claurst /home/claurst/.ssh/authorized_keys
        if [[ "${_invalid}" -gt 0 ]]; then
            echo "[entrypoint] Installed ${_valid} authorized key(s); ${_invalid} invalid key(s) were skipped."
        else
            echo "[entrypoint] Installed ${_valid} authorized key(s)."
        fi
    else
        rm -f "${_tmp_keys}"
        echo "[entrypoint] ERROR: No valid SSH keys found in AUTHORIZED_KEYS." >&2
        echo "[entrypoint]        Logins will be rejected." >&2
    fi
    unset _tmp_line _tmp_keys _valid _invalid _line
else
    echo "[entrypoint] WARNING: AUTHORIZED_KEYS is not set." >&2
    echo "[entrypoint]          No SSH public keys installed; logins will be rejected." >&2
    echo "[entrypoint]          Pass -e AUTHORIZED_KEYS=\"\$(cat ~/.ssh/id_ed25519.pub)\" to docker run." >&2
fi

# ── Start SSH daemon ──────────────────────────────────────────────────────────
echo "[entrypoint] Starting sshd..."
exec /usr/sbin/sshd -D -e
