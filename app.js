require("dotenv").config();
const fs = require("fs");
const path = require("path");
const axios = require("axios");
const chokidar = require("chokidar");

// Configuration from environment variables
const WATCH_DIR = process.env.WATCH_DIR || "/app/data";
const TARGET_ENDPOINT =
  process.env.TARGET_ENDPOINT || "http://192.168.178.10/api/podping";
const MAX_CONCURRENT_REQUESTS = parseInt(
  process.env.MAX_CONCURRENT_REQUESTS || "5"
);
const REQUEST_TIMEOUT_MS = parseInt(process.env.REQUEST_TIMEOUT_MS || "30000");
const REQUEST_RETRY_COUNT = parseInt(process.env.REQUEST_RETRY_COUNT || "3");
const REQUEST_RETRY_DELAY_MS = parseInt(
  process.env.REQUEST_RETRY_DELAY_MS || "3000"
);
const FILE_AGE_TIMEOUT_SEC = parseInt(process.env.FILE_AGE_TIMEOUT_SEC || "60");

// Add new environment variables
const CONFIG_FILE = process.env.CONFIG_FILE || "/app/conf/bp-config.toml";
const RESTART_MINUTES = parseInt(process.env.RESTART_MINUTES || "3");

// Create a request queue to manage concurrent requests
class RequestQueue {
  constructor(maxConcurrent) {
    this.maxConcurrent = maxConcurrent;
    this.running = 0;
    this.queue = [];
  }

  async add(task) {
    if (this.running >= this.maxConcurrent) {
      // Queue the task if we're at max concurrent requests
      await new Promise((resolve) => this.queue.push(resolve));
    }

    this.running++;
    try {
      await task();
    } finally {
      this.running--;
      if (this.queue.length > 0) {
        // Process next queued task
        const next = this.queue.shift();
        next();
      }
    }
  }
}

const requestQueue = new RequestQueue(MAX_CONCURRENT_REQUESTS);

// Add function to restart podpingd
async function restartPodpingd() {
  try {
    // Calculate new start time (X minutes ago)
    const startTime = new Date(Date.now() - RESTART_MINUTES * 60 * 1000);
    const formattedTime = startTime.toISOString().replace("Z", "+0000");

    console.log(`Restarting podpingd with start time: ${formattedTime}`);

    // Read current config
    let configContent = await fs.promises.readFile(CONFIG_FILE, "utf8");

    // Check if the line exists (commented or uncommented)
    const hasStartDateTime = configContent.match(/^#?\s*start_datetime.*$/m);

    if (hasStartDateTime) {
      // Replace existing line (commented or uncommented)
      configContent = configContent.replace(
        /^#?\s*start_datetime.*$/m,
        `start_datetime = "${formattedTime}"`
      );
    } else {
      // Add new line if it doesn't exist
      configContent += `\nstart_datetime = "${formattedTime}"\n`;
    }

    // Write updated config
    await fs.promises.writeFile(CONFIG_FILE, configContent);

    // Use sudo to restart podpingd
    const { exec } = require("child_process");
    await new Promise((resolve, reject) => {
      exec("sudo supervisorctl restart podpingd", (error, stdout, stderr) => {
        if (error) {
          console.error("Error restarting podpingd:", error);
          reject(error);
          return;
        }
        console.log("Podpingd restart output:", stdout);
        resolve();
      });
    });
  } catch (error) {
    console.error("Failed to restart podpingd:", error);
  }
}

let consecutiveFailures = 0;
const MAX_CONSECUTIVE_FAILURES = parseInt(
  process.env.MAX_CONSECUTIVE_FAILURES || "5"
);

async function processJsonFile(filePath) {
  console.log(`Processing file: ${filePath}`);

  try {
    // Read and validate JSON
    const content = await fs.promises.readFile(filePath, "utf8");
    const jsonData = JSON.parse(content);

    const reason = jsonData.reason || "unknown";
    const rss = Array.isArray(jsonData.iris) ? jsonData.iris.join(",") : "";

    console.log(`DEBUG: Reason: ${reason}`);
    console.log(`DEBUG: RSS: ${rss}`);
    console.log(`DEBUG: Making request to ${TARGET_ENDPOINT}/${reason}`);

    // Add request to queue
    await requestQueue.add(async () => {
      try {
        const response = await axios({
          method: "post",
          url: `${TARGET_ENDPOINT}/${reason}`,
          headers: {
            "Content-Type": "application/json",
            "Podcast-RSS": rss,
          },
          timeout: REQUEST_TIMEOUT_MS,
          maxRetries: REQUEST_RETRY_COUNT,
          retryDelay: REQUEST_RETRY_DELAY_MS,
        });
        console.log(
          `SUCCESS: Processed ${filePath} with status ${response.status}`
        );
        consecutiveFailures = 0; // Reset counter on success
      } catch (error) {
        console.error(`ERROR: Request failed for ${filePath}:`, error.message);

        // Only increment failures for podpingd-related issues
        // Network errors should not trigger podpingd restarts
        if (!error.isAxiosError || error.code !== "ECONNREFUSED") {
          consecutiveFailures++;

          if (consecutiveFailures >= MAX_CONSECUTIVE_FAILURES) {
            console.error(
              `${MAX_CONSECUTIVE_FAILURES} consecutive podping-related failures detected. Restarting podpingd...`
            );
            await restartPodpingd();
            consecutiveFailures = 0; // Reset after restart
          }
        } else {
          console.error("Network connection error - will retry on next file");
        }
      }
    });
  } catch (error) {
    if (error.name === "SyntaxError") {
      console.error(`ERROR: Invalid JSON in file: ${filePath}`);
      console.error("File content:");
      console.error(await fs.promises.readFile(filePath, "utf8"));
    } else {
      console.error(`ERROR: Failed to process ${filePath}:`, error.message);
    }
  }
}

// Ensure watch directory exists
fs.mkdirSync(WATCH_DIR, { recursive: true });

// Track last file modification time
let lastFileTime = Date.now();

async function waitForFileStability(
  filePath,
  timeout = 1000,
  checkInterval = 50
) {
  const startTime = Date.now();
  let lastSize = -1;
  let lastModified = -1;

  while (Date.now() - startTime < timeout) {
    try {
      const stats = await fs.promises.stat(filePath);

      // If file size and modification time haven't changed since last check,
      // and we've checked at least once before, file is likely stable
      if (
        lastSize === stats.size &&
        lastModified === stats.mtimeMs &&
        lastSize !== -1
      ) {
        return true;
      }

      lastSize = stats.size;
      lastModified = stats.mtimeMs;

      // Wait for next check
      await new Promise((resolve) => setTimeout(resolve, checkInterval));
    } catch (error) {
      // File might have been deleted
      return false;
    }
  }

  // Timeout reached
  console.warn(`Warning: File stability timeout reached for ${filePath}`);
  return true;
}

// Replace the fs.watch setup with chokidar
const watcher = chokidar.watch(WATCH_DIR, {
  persistent: true,
  ignoreInitial: true,
  awaitWriteFinish: {
    stabilityThreshold: 1000,
    pollInterval: 50,
  },
});

watcher.on("add", async (filePath) => {
  if (!filePath.endsWith(".json")) return;

  lastFileTime = Date.now();

  // Check if file still exists (might have been deleted)
  if (fs.existsSync(filePath)) {
    await processJsonFile(filePath);
  }
});

// Process existing files on startup
async function processExistingFiles() {
  try {
    const files = await fs.promises.readdir(WATCH_DIR);
    for (const file of files) {
      if (file.endsWith(".json")) {
        await processJsonFile(path.join(WATCH_DIR, file));
      }
    }
  } catch (error) {
    console.error("Error processing existing files:", error);
  }
}

// Monitor for file age timeout
setInterval(() => {
  const timeSinceLastFile = (Date.now() - lastFileTime) / 1000;
  if (timeSinceLastFile > FILE_AGE_TIMEOUT_SEC) {
    console.log(
      `INFO: No new files in the last ${FILE_AGE_TIMEOUT_SEC} seconds. This is normal if there are no new podping updates.`
    );
    // Only restart if it's been a very long time (e.g., 1 hour)
    if (timeSinceLastFile > 3600) {
      // 1 hour
      console.log("WARNING: No files for over an hour. Restarting podpingd...");
      restartPodpingd();
    }
  }
}, FILE_AGE_TIMEOUT_SEC * 1000);

// Handle process termination
function cleanup() {
  watcher.close();
  process.exit(0);
}

process.on("SIGTERM", cleanup);
process.on("SIGINT", cleanup);

// Start processing
console.log(`Starting file watch on ${WATCH_DIR}`);
processExistingFiles();
