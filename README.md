# podpingd-post

This project combines `podpingd` (a program that monitors the Hive blockchain for podcast updates) with a Node.js watcher that forwards these updates to an API endpoint as a POST request.

## Components

- **podpingd**: Monitors the Hive blockchain and writes JSON files for each podping
- **Node.js watcher**: Monitors the JSON files and forwards them to a configured API endpoint

## Configuration

Copy the example environment file and modify it to your needs:

```shell
cp .env.example .env
```

Available environment variables:

```env
# Directory where podpingd writes JSON files
WATCH_DIR=/app/data

# API endpoint for sending podping notifications
TARGET_ENDPOINT=http://your-api-endpoint/api/podping

# Request configuration
MAX_CONCURRENT_REQUESTS=5      # Maximum concurrent HTTP requests
REQUEST_TIMEOUT_MS=30000       # HTTP request timeout in milliseconds
REQUEST_RETRY_COUNT=3          # Number of retries for failed requests
REQUEST_RETRY_DELAY_MS=3000    # Delay between retries in milliseconds

# File monitoring configuration
FILE_AGE_TIMEOUT_SEC=60        # Restart podpingd if no new files in this period

# Podpingd configuration
CONFIG_FILE=/app/conf/post-config.toml  # podpingd config file location
RESTART_MINUTES=3              # How far back to set start_datetime when restarting
MAX_CONSECUTIVE_FAILURES=5     # Restart after this many consecutive HTTP failures
```

## Running with Docker

Build and run the container:

```shell
docker build -t podpingd-post .
docker run -d \
  --name podpingd-post \
  --env-file .env \
  podpingd-post
```

## Monitoring

The application logs to stdout/stderr and can be monitored using:

```shell
docker logs -f podping-watcher
```

## Error Handling

The watcher automatically restarts podpingd when:

- No new files are detected for FILE_AGE_TIMEOUT_SEC seconds
- MAX_CONSECUTIVE_FAILURES HTTP requests fail in a row

When restarting, it sets podpingd's start_datetime to RESTART_MINUTES in the past to catch up on missed podpings.

## Development

The project uses:

- Node.js for the file watcher and HTTP requests
- Rust for podpingd (the blockchain monitor)
- Supervisor for process management

To modify the Node.js watcher, edit `app.js`. For podpingd configuration, modify the TOML file specified in CONFIG_FILE.
