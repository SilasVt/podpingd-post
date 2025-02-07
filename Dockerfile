# Use Ubuntu as base image
FROM ubuntu:22.04

# Avoid prompts during package installation
ENV DEBIAN_FRONTEND=noninteractive

# Update and install essential packages
RUN apt-get update && apt-get install -y \
    curl \
    jq \
    build-essential \
    cargo \
    pkg-config \
    capnproto \
    supervisor \
    inotify-tools \
    ca-certificates \
    libssl-dev \
    nodejs \
    && rm -rf /var/lib/apt/lists/*

# Install Rust and cargo (requires HTTPS)
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
ENV PATH="/root/.cargo/bin:${PATH}"

# Create dedicated non-root user and group
RUN useradd -ms /bin/bash podping

# Create needed directories (as root)
RUN mkdir -p /var/log/supervisor

# Copy inotify script & make executable
COPY inotify.sh /usr/local/bin/inotify.sh
RUN chmod +x /usr/local/bin/inotify.sh

# Copy monitoring script & make executable
COPY check_podpingd.sh /usr/local/bin/check_podpingd.sh
RUN chmod +x /usr/local/bin/check_podpingd.sh

# Create a startup script that sets inotify limits
RUN echo '#!/bin/bash\n\
echo 524288 > /proc/sys/fs/inotify/max_user_watches\n\
echo 524288 > /proc/sys/fs/inotify/max_queued_events\n\
exec "$@"' > /usr/local/bin/docker-entrypoint.sh && \
    chmod +x /usr/local/bin/docker-entrypoint.sh

# Configure supervisord
RUN mkdir -p /etc/supervisor/conf.d
COPY supervisord.conf /etc/supervisor/conf.d/supervisord.conf

# Set working directory
WORKDIR /app

# Initialize Node.js project and install dependencies
COPY package*.json ./
RUN npm install

# Copy source files into the container
COPY . .

# Build the Rust project (as root)
RUN cargo build --release

# Create data directory for podpingd
RUN mkdir -p /app/data

# Adjust ownership for all relevant directories and files
RUN chown -R podping:podping /app \
    /usr/local/bin/inotify.sh \
    /usr/local/bin/check_podpingd.sh \
    /var/log/supervisor \
    /etc/supervisor/conf.d

# Switch to non-root user
USER podping

# Use the entrypoint script
ENTRYPOINT ["/usr/local/bin/docker-entrypoint.sh"]
CMD ["/usr/bin/supervisord", "-c", "/etc/supervisor/conf.d/supervisord.conf"] 