FROM debian:bookworm-slim

# TARGETARCH is injected by Docker Buildx (amd64 or arm64).
ARG TARGETARCH

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        openssh-server \
        ca-certificates \
        libssl3 \
        libasound2 \
    && rm -rf /var/lib/apt/lists/* \
    && mkdir -p /var/run/sshd

# Create the user that SSH clients authenticate as.
RUN useradd -m -s /bin/bash claurst \
    && mkdir -p /home/claurst/.ssh \
    && chmod 700 /home/claurst/.ssh \
    && chown -R claurst:claurst /home/claurst/.ssh

# Copy the pre-built binary for the target platform.
# The build workflow places binaries at docker/bin/<arch>/claurst.
COPY bin/${TARGETARCH}/claurst /usr/local/bin/claurst
RUN chmod +x /usr/local/bin/claurst

# SSH daemon configuration.
COPY sshd_config /etc/ssh/sshd_config

# Container entrypoint (generates host keys, installs authorized_keys, starts sshd).
COPY entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh

EXPOSE 22

ENTRYPOINT ["/entrypoint.sh"]
