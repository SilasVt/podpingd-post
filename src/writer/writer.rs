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
use crate::config::Settings;
use crate::hive::scanner::HiveBlockWithNum;
use color_eyre::eyre::Error;
use color_eyre::Result;
use tokio::sync::broadcast::Receiver;

pub(crate) trait Writer {
    async fn new(settings: &Settings) -> Self
    where
        Self: Sized;
    async fn get_last_block(&self) -> Result<Option<u64>, Error>;
    fn start(
        &self,
        rx: Receiver<HiveBlockWithNum>,
    ) -> impl std::future::Future<Output = Result<(), Error>> + Send;
    fn start_batch(
        &self,
        rx: Receiver<Vec<HiveBlockWithNum>>,
    ) -> impl std::future::Future<Output = Result<(), Error>> + Send;
}

pub const LAST_UPDATED_BLOCK_FILENAME: &str = "last_updated_block";
