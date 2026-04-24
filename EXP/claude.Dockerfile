FROM alpine:latest

# labels
LABEL maintainer="Adam Bajger"
LABEL description="Claude CLI in an alpine container to deploy in cloud run or similar services"

RUN apk add libgcc libstdc++ ripgrep curl bash

# Ensure `claude`'s local bin is on PATH for non-login shells
ENV PATH="/home/claude/.local/bin:${PATH}"

# create a non-root user to run the application (alpine syntax)
RUN addgroup -S claude && adduser -S -G claude -h /home/claude -s /bin/bash claude && \
    mkdir -p /home/claude && chown -R claude:claude /home/claude
USER claude
ENV HOME=/home/claude
WORKDIR /home/claude

# Install `uv` as the non-root `claude` user so it lands in the user's home
# and is owned by that user.
RUN curl -Ls https://astral.sh/uv/install.sh | sh

RUN curl -fsSL https://claude.ai/install.sh | bash
RUN <<EOF
    set -e
    # ensure the local bin is in the PATH for the current user
    echo 'export PATH="$HOME/.local/bin:${PATH}"' >> /home/claude/.bashrc
    echo 'export USE_BUILTIN_RIPGREP=0' >> /home/claude/.bashrc
EOF

# set bash as the default shell
SHELL ["/bin/bash", "-c"]


# ENTRYPOINT ["/usr/local/bin/claude"]
# CMD ["--help"]

# docker build -t adambajger/claude-cli-cloud-run -f EXP/claude.Dockerfile EXP