/*
 * Copyright (c) 2024 Gates Solutions LLC.
 *
 *      This file is part of podpingd.
 *
 *     podpingd is free software: you can redistribute it and/or modify it under the terms of the GNU Lesser General Public License as published by the Free Software Foundation, either version 3 of the License, or (at your option) any later version.
 *
 *     podpingd is distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the GNU Lesser General Public License for more details.
 *
 *     You should have received a copy of the GNU Lesser General Public License along with podpingd. If not, see <https://www.gnu.org/licenses/>.
 */
use crate::config::{Settings, WriterUrlStyle};
use crate::hive::scanner::HiveBlockWithNum;
use crate::writer::writer::{Writer, LAST_UPDATED_BLOCK_FILENAME};
use chrono::{Datelike, Timelike};
use color_eyre::eyre::Error;
use color_eyre::Result;
use podping_schemas::org::podcastindex::podping::podping_json::Podping;
use reqwest::{Client, Response, StatusCode};
use rusty_s3::{Bucket, Credentials, S3Action, UrlStyle};
use std::env;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::broadcast::Receiver;
use tokio::task::JoinSet;
use tracing::{debug, error, info, warn};
use url::Url;

const CONTENT_TYPE_APPLICATION_JSON: &'static str = "application/json";
const CONTENT_TYPE_TEXT_PLAIN: &'static str = "text/plain";
const ONE_MINUTE: Duration = Duration::from_secs(60);

#[derive(Error, Debug)]
pub enum HeadBucketError {
    #[error("Bucket not found")]
    NotFound,
    #[error("Permission denied accessing bucket")]
    AccessDenied,
    #[error("Bad request accessing bucket")]
    BadRequest,
    #[error("Unknown error accessing bucket")]
    UnknownError,
}

async fn head_bucket(osw: &ObjectStorageWriter) -> Result<Response, HeadBucketError> {
    let action = osw.bucket.head_bucket(Some(&osw.credentials));
    let url = action.sign(ONE_MINUTE);

    debug!("head_bucket_url: {:?}", url.clone().to_string());

    // TODO: Add retry logic
    let response = match osw.http_client.head(url).send().await {
        Ok(exists) => exists,
        Err(_) => return Err(HeadBucketError::UnknownError),
    };

    let status = response.status();

    match status {
        StatusCode::OK => {
            debug!("Successfully connected to bucket.");
            Ok(response)
        }
        StatusCode::NOT_FOUND => Err(HeadBucketError::NotFound),
        StatusCode::FORBIDDEN => Err(HeadBucketError::AccessDenied),
        StatusCode::BAD_REQUEST => Err(HeadBucketError::BadRequest),
        _ => Err(HeadBucketError::UnknownError),
    }
}

#[derive(Error, Debug)]
pub enum GetObjectError {
    #[error("Object not found")]
    NotFound,
    #[error("Permission denied accessing object")]
    AccessDenied,
    #[error("Bad request accessing object")]
    BadRequest,
    #[error("Unknown error accessing object")]
    UnknownError,
}

async fn get_object(osw: &ObjectStorageWriter, path: PathBuf) -> Result<Response, GetObjectError> {
    let path_str = path.to_string_lossy();
    let mut action = osw.bucket.get_object(Some(&osw.credentials), &path_str);
    action
        .query_mut()
        .insert("response-cache-control", "no-cache, no-store");
    let url = action.sign(ONE_MINUTE);

    debug!("get_object_url: {:?}", url.clone().to_string());

    // TODO: Add retry logic
    let response = match osw.http_client.get(url).send().await {
        Ok(response) => response,
        Err(_) => return Err(GetObjectError::UnknownError),
    };

    let status = response.status();

    debug!(
        "bucket: {}, path: {}, get_object_status: {:?}",
        &osw.bucket.name(),
        path_str,
        status
    );

    match status {
        StatusCode::OK => Ok(response),
        StatusCode::NOT_FOUND => Err(GetObjectError::NotFound),
        StatusCode::FORBIDDEN => Err(GetObjectError::AccessDenied),
        StatusCode::BAD_REQUEST => Err(GetObjectError::BadRequest),
        _ => Err(GetObjectError::UnknownError),
    }
}

#[derive(Error, Debug)]
pub enum PutObjectError {
    #[error("Permission denied writing object")]
    AccessDenied,
    #[error("Bad request writing object")]
    BadRequest,
    #[error("Unknown error writing object")]
    UnknownError,
}

async fn put_object(
    bucket: Arc<Bucket>,
    credentials: Arc<Credentials>,
    http_client: Arc<Client>,
    path: PathBuf,
    body: String,
    content_type: Option<String>,
) -> Result<Response, PutObjectError> {
    let path_str = path.to_string_lossy();
    let action = bucket.put_object(Some(&credentials), &path_str);
    let url = action.sign(ONE_MINUTE);

    debug!("put_object_url: {:?}", url.clone().to_string());

    let content_type_str = content_type.unwrap_or_else(|| CONTENT_TYPE_TEXT_PLAIN.to_string());

    // TODO: Add retry logic
    let response = match http_client
        .clone()
        .put(url)
        .header("Content-Type", content_type_str)
        .body(body)
        .send()
        .await
    {
        Ok(response) => response,
        Err(_) => return Err(PutObjectError::UnknownError),
    };

    let status = response.status();

    debug!(
        "bucket: {}, path: {}, put_object_status: {:?}",
        bucket.name(),
        path_str,
        status
    );

    match status {
        StatusCode::OK => Ok(response),
        StatusCode::FORBIDDEN => Err(PutObjectError::AccessDenied),
        StatusCode::BAD_REQUEST => Err(PutObjectError::BadRequest),
        _ => Err(PutObjectError::UnknownError),
    }
}

async fn object_storage_write_block_transactions(
    bucket: Arc<Bucket>,
    credentials: Arc<Credentials>,
    http_client: Arc<Client>,
    block: HiveBlockWithNum,
) -> Result<(), Error> {
    if block.transactions.is_empty() {
        info!("No Podpings for block {}", block.block_num);
    } else {
        let current_block_path = PathBuf::new()
            .join(block.timestamp.year().to_string())
            .join(block.timestamp.month().to_string())
            .join(block.timestamp.day().to_string())
            .join(block.timestamp.hour().to_string())
            .join(block.timestamp.minute().to_string())
            .join(block.timestamp.second().to_string());

        let mut write_join_set = JoinSet::new();

        for tx in &block.transactions {
            for (i, podping) in tx.podpings.iter().enumerate() {
                let podping_file = match podping {
                    Podping::V0(_) | Podping::V02(_) | Podping::V03(_) | Podping::V10(_) => {
                        current_block_path
                            .join(format!("{}_{}_{}.json", block.block_num, tx.tx_id, i))
                    }
                    Podping::V11(pp) => current_block_path.join(format!(
                        "{}_{}_{}_{}.json",
                        block.block_num,
                        tx.tx_id,
                        pp.session_id.to_string(),
                        pp.timestamp_ns.to_string()
                    )),
                };

                let json = serde_json::to_string(&podping);

                match json {
                    Ok(json) => {
                        info!(
                            "block: {}, tx: {}, podping: {}",
                            block.block_num, tx.tx_id, json
                        );

                        info!(
                            "Writing podping to object storage: {}",
                            podping_file.to_string_lossy()
                        );

                        write_join_set.spawn(put_object(
                            bucket.clone(),
                            credentials.clone(),
                            http_client.clone(),
                            podping_file,
                            json,
                            Some(CONTENT_TYPE_APPLICATION_JSON.to_string()),
                        ));
                    }
                    Err(e) => {
                        error!(
                            "Error writing podping file {}: {}",
                            podping_file.to_string_lossy(),
                            e
                        );
                    }
                }
            }
        }

        write_join_set.join_all().await;
    }
    Ok(())
}

async fn object_storage_write_last_block(
    osw: &ObjectStorageWriter,
    block_num: u64,
) -> Result<(), Error> {
    let path = PathBuf::from(LAST_UPDATED_BLOCK_FILENAME);
    let block_num_str = block_num.to_string();
    let response = put_object(
        osw.bucket.clone(),
        osw.credentials.clone(),
        osw.http_client.clone(),
        path,
        block_num_str,
        Some(CONTENT_TYPE_TEXT_PLAIN.to_string()),
    )
    .await;

    match response {
        Ok(_) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

pub(crate) struct ObjectStorageWriter {
    bucket: Arc<Bucket>,
    credentials: Arc<Credentials>,
    http_client: Arc<Client>,
}

impl Writer for ObjectStorageWriter {
    async fn new(settings: &Settings) -> Self
    where
        Self: Sized,
    {
        let base_url_result = match settings.writer.object_storage_base_url.clone() {
            Some(base_url) => base_url,
            None => panic!("object_storage_base_url is not set"),
        }
        .parse::<Url>();

        let access_key = match env::var("AWS_ACCESS_KEY_ID") {
            Ok(access_key) => access_key,
            Err(e) => panic!("AWS_ACCESS_KEY_ID is not set: {}", e),
        };

        let access_secret = match env::var("AWS_SECRET_ACCESS_KEY") {
            Ok(access_secret) => access_secret,
            Err(e) => panic!("AWS_SECRET_ACCESS_KEY is not set: {}", e),
        };

        let credentials = Arc::new(Credentials::new(access_key, access_secret));

        let base_url = match base_url_result {
            Ok(base_url) => base_url,
            Err(e) => panic!("Error parsing object storage base URL: {}", e),
        };

        let bucket_name = match settings.writer.object_storage_bucket_name.clone() {
            Some(bucket_name) => bucket_name,
            None => panic!("object_storage_bucket_name is not set"),
        };

        let region = match settings.writer.object_storage_region.clone() {
            Some(region) => region,
            None => panic!("object_storage_region is not set"),
        };

        let url_style = match settings.writer.object_storage_url_style {
            Some(WriterUrlStyle::Path) => UrlStyle::Path,
            Some(WriterUrlStyle::VirtualHost) => UrlStyle::VirtualHost,
            None => panic!("object_storage_url_style is not set"),
        };

        let bucket = match Bucket::new(base_url, url_style, bucket_name.clone(), region) {
            Ok(client) => Arc::new(client),
            Err(e) => panic!("Error creating S3 client: {}", e),
        };

        let http_client = Arc::new(Client::new());

        let osw = ObjectStorageWriter {
            bucket,
            credentials,
            http_client,
        };

        match head_bucket(&osw).await {
            Ok(_) => osw,
            Err(e) => panic!("Error accessing bucket {}: {}", bucket_name, e),
        }
    }

    async fn get_last_block(&self) -> Result<Option<u64>, Error> {
        let path = PathBuf::from(LAST_UPDATED_BLOCK_FILENAME);
        let response = get_object(self, path).await;

        match response {
            Ok(r) => match r.text().await {
                Ok(s) => match s.trim().parse::<u64>() {
                    Ok(block) => Ok(Some(block)),
                    _ => Ok(None),
                },
                _ => Ok(None),
            },
            Err(GetObjectError::NotFound) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    async fn start(&self, mut rx: Receiver<HiveBlockWithNum>) -> Result<(), Error> {
        loop {
            let result = rx.recv().await;

            let block = match result {
                Ok(block) => Some(block),
                Err(RecvError::Lagged(e)) => {
                    warn!("Object Storage writer is lagging: {}", e);

                    None
                }
                Err(RecvError::Closed) => {
                    panic!("Object Storage writer channel closed");
                }
            };

            match block {
                Some(block) => {
                    let block_num = block.block_num.to_owned();

                    object_storage_write_block_transactions(
                        self.bucket.clone(),
                        self.credentials.clone(),
                        self.http_client.clone(),
                        block,
                    )
                    .await?;
                    object_storage_write_last_block(self, block_num).await?
                }
                None => {}
            }
        }
    }

    async fn start_batch(&self, mut rx: Receiver<Vec<HiveBlockWithNum>>) -> Result<(), Error> {
        loop {
            let result = rx.recv().await;

            let block = match result {
                Ok(block) => Some(block),
                Err(RecvError::Lagged(e)) => {
                    warn!("Object Storage writer is lagging: {}", e);

                    None
                }
                Err(RecvError::Closed) => break,
            };

            match block {
                Some(blocks) => {
                    let last_block_num = blocks.last().unwrap().block_num;
                    let mut write_join_set = JoinSet::new();

                    for block in blocks {
                        write_join_set.spawn(object_storage_write_block_transactions(
                            self.bucket.clone(),
                            self.credentials.clone(),
                            self.http_client.clone(),
                            block,
                        ));
                    }

                    write_join_set.join_all().await;

                    object_storage_write_last_block(self, last_block_num).await?;
                }
                None => {}
            }
        }

        Ok(())
    }
}
