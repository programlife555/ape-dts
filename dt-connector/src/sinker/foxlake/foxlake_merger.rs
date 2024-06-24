use std::{
    sync::{Arc, Mutex},
    time::Instant,
};

use anyhow::Context;
use async_trait::async_trait;
use dt_common::{
    config::s3_config::S3Config,
    log_error,
    meta::dt_data::{DtData, DtItem},
    monitor::monitor::Monitor,
};
use rusoto_s3::S3Client;
use sqlx::{MySql, Pool};

use crate::{close_conn_pool, sinker::base_sinker::BaseSinker, Sinker};

pub struct FoxlakeMerger {
    pub batch_size: usize,
    pub monitor: Arc<Mutex<Monitor>>,
    pub s3_client: S3Client,
    pub s3_config: S3Config,
    pub conn_pool: Pool<MySql>,
}

#[async_trait]
impl Sinker for FoxlakeMerger {
    async fn sink_raw(&mut self, data: Vec<DtItem>, _batch: bool) -> anyhow::Result<()> {
        if data.is_empty() {
            return Ok(());
        }

        self.batch_sink(data).await
    }

    async fn close(&mut self) -> anyhow::Result<()> {
        return close_conn_pool!(self);
    }
}

impl FoxlakeMerger {
    async fn batch_sink(&mut self, data: Vec<DtItem>) -> anyhow::Result<()> {
        let start_time = Instant::now();

        let (all_data_size, all_row_count) = self.batch_merge(data).await?;

        BaseSinker::update_batch_monitor(
            &mut self.monitor,
            all_row_count,
            all_data_size,
            start_time,
        )
        .await
    }

    pub async fn batch_merge(&mut self, data: Vec<DtItem>) -> anyhow::Result<(usize, usize)> {
        log_error!("1111111111111111111111111");

        let mut all_row_count = 0;
        let mut all_data_size = 0;
        let mut schema = String::new();
        let mut tb = String::new();
        let mut s3_files = Vec::new();
        let mut insert_only = true;

        for dt_item in data {
            if let DtData::Foxlake { file_meta } = dt_item.dt_data {
                all_row_count += file_meta.row_count;
                all_data_size += file_meta.data_size;
                schema = file_meta.schema;
                tb = file_meta.tb;
                s3_files.push(file_meta.data_file_name);
                insert_only &= file_meta.insert_only;
            }
        }

        let s3 = &self.s3_config;
        let files: Vec<String> = s3_files.iter().map(|i| format!("'{}'", i)).collect();
        let insert_only = if insert_only { "TRUE" } else { "FALSE" };
        let sql = format!(
            r#"MERGE INTO TABLE `{}`.`{}` 
            USING URI = '{}/' 
            ENDPOINT = '{}' 
            CREDENTIALS = (ACCESS_KEY_ID='{}' SECRET_ACCESS_KEY='{}') 
            FILES=({}) FILE_FORMAT = (TYPE='DML_CHANGE_LOG') INSERT_ONLY = {};"#,
            schema,
            tb,
            s3.root_url,
            s3.endpoint,
            s3.access_key,
            s3.secret_key,
            files.join(","),
            insert_only
        );

        log_error!("{}", sql);

        let query = sqlx::query(&sql);
        query
            .execute(&self.conn_pool)
            .await
            .with_context(|| format!("merge to foxlake failed: {}", sql))?;

        log_error!("22222222222222222222222222");

        Ok((all_data_size, all_row_count))
    }
}
