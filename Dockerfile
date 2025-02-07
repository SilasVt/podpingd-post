# Use Ubuntu as base image
FROM ubuntu:22.04

# Avoid prompts during package installation
ENV DEBIAN_FRONTEND=noninteractive

# Update and install essential packages
RUN apt-get update && apt-get install -y \
    curl \
    build-essential \
    pkg-config \
    capnproto \
    supervisor \
    ca-certificates \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Install Node.js and npm
RUN curl -fsSL https://deb.nodesource.com/setup_18.x | bash - \
    && apt-get install -y nodejs \
    && rm -rf /var/lib/apt/lists/*

# Create dedicated non-root user and group
RUN useradd -ms /bin/bash podping

# Install Rust for podping user
USER podping
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
ENV PATH="/home/podping/.cargo/bin:${PATH}"
USER root

# Create needed directories (as root)
RUN mkdir -p /var/log/supervisor

# Create a startup script that sets inotify limits
RUN echo '#!/bin/bash\n\
exec "$@"' > /usr/local/bin/docker-entrypoint.sh && \
    chmod +x /usr/local/bin/docker-entrypoint.sh

# Configure supervisord
RUN mkdir -p /etc/supervisor/conf.d
COPY supervisord.conf /etc/supervisor/conf.d/supervisord.conf

# Create log directory with correct permissions
RUN mkdir -p /var/log/supervisor && \
    chown -R podping:podping /var/log/supervisor && \
    chmod 755 /var/log/supervisor

# Set working directory
WORKDIR /app

# Initialize Node.js project and install dependencies
COPY package*.json ./
RUN npm install

# Copy source files into the container
COPY . .

# Set ownership of all files to podping user
RUN chown -R podping:podping /app

# Build the Rust project (as root)
USER podping
RUN cargo build --release
USER root

# Create data directory for podpingd
RUN mkdir -p /app/data

# Adjust ownership for supervisor directories
RUN chown -R podping:podping \
    /var/log/supervisor \
    /etc/supervisor/conf.d

# Make sure supervisor can write its pid file
RUN mkdir -p /var/run && chown podping:podping /var/run

# Use the entrypoint script
ENTRYPOINT ["/usr/local/bin/docker-entrypoint.sh"]
CMD ["/usr/bin/supervisord", "-c", "/etc/supervisor/conf.d/supervisord.conf"] 