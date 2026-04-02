use std::{
    collections::{HashSet, VecDeque},
    fs,
    path::PathBuf,
};

use aliyun_oss_client::{
    decode::{ListError, RefineBucket, RefineBucketList},
    decode::{RefineObject, RefineObjectList},
    object::InitObject,
    file::Files,
    BucketName, Client, EndPoint, KeyId, KeySecret,
};
use anyhow::{anyhow, Context};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

const DEFAULT_REGION: &str = "oss-cn-hongkong";
const MAX_PREFIX_TOTAL_ITEMS: usize = 300_000;
const MAX_PREFIX_SCAN_PARENTS: usize = 30_000;
const MAX_OBJECT_SCAN_PAGES: usize = 5000;
const MAX_OBJECT_SCAN_KEYS: usize = 1_000_000;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct LocalConfig {
    access_key_id: String,
    access_key_secret: String,
    bucket: String,
    region: String,
    prefix: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PublicConfig {
    bucket: String,
    region: String,
    prefix: String,
    has_oss_config: bool,
    config_path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveConfigPayload {
    access_key_id: Option<String>,
    access_key_secret: Option<String>,
    bucket: Option<String>,
    region: Option<String>,
    prefix: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SaveConfigResponse {
    ok: bool,
    config: PublicConfig,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UploadFilePayload {
    name: String,
    bytes: Vec<u8>,
    mime_type: String,
    size: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UploadLocalPathPayload {
    path: String,
    name: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UploadItem {
    name: String,
    key: String,
    prefix: String,
    url: Option<String>,
    size: u64,
    ok: bool,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UploadResponse {
    results: Vec<UploadItem>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PrefixesResponse {
    prefixes: Vec<String>,
    parent_prefix: String,
    default_prefix: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AllPrefixesStats {
    visited: usize,
    fallback_hits: usize,
    failed_parents: Vec<String>,
    truncated: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AllPrefixesResponse {
    prefixes: Vec<String>,
    default_prefix: String,
    stats: AllPrefixesStats,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BucketSummary {
    name: String,
    region: String,
}

#[derive(Debug, Default)]
struct BucketSummaryItem {
    name: String,
    region: String,
}

#[derive(Debug, Default)]
struct BucketSummaryList {
    buckets: Vec<BucketSummaryItem>,
}

#[derive(Debug, Default)]
struct PrefixFallbackItem;

#[derive(Debug, Default)]
struct PrefixFallbackList {
    prefixes: Vec<String>,
    next_continuation_token: String,
}

#[derive(Debug, Default)]
struct ObjectKeyFallbackItem {
    key: String,
}

#[derive(Debug, Default)]
struct ObjectKeyFallbackList {
    keys: Vec<String>,
    next_continuation_token: String,
}

#[derive(Debug)]
struct BucketListDecodeError;

#[derive(Debug)]
struct PrefixListDecodeError;

impl std::fmt::Display for BucketListDecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        "bucket list decode failed".fmt(f)
    }
}

impl std::error::Error for BucketListDecodeError {}

impl ListError for BucketListDecodeError {}

impl std::fmt::Display for PrefixListDecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        "prefix list decode failed".fmt(f)
    }
}

impl std::error::Error for PrefixListDecodeError {}

impl ListError for PrefixListDecodeError {}

impl RefineBucket<BucketListDecodeError> for BucketSummaryItem {
    fn set_name(&mut self, name: &str) -> Result<(), BucketListDecodeError> {
        self.name = name.to_string();
        Ok(())
    }

    fn set_location(&mut self, location: &str) -> Result<(), BucketListDecodeError> {
        self.region = normalize_bucket_region(location);
        Ok(())
    }
}

impl RefineBucketList<BucketSummaryItem, BucketListDecodeError> for BucketSummaryList {
    fn set_list(&mut self, list: Vec<BucketSummaryItem>) -> Result<(), BucketListDecodeError> {
        self.buckets = list;
        Ok(())
    }
}

impl InitObject<BucketSummaryItem> for BucketSummaryList {
    fn init_object(&mut self) -> Option<BucketSummaryItem> {
        Some(BucketSummaryItem::default())
    }
}

impl RefineObject<PrefixListDecodeError> for PrefixFallbackItem {}

impl RefineObjectList<PrefixFallbackItem, PrefixListDecodeError> for PrefixFallbackList {
    fn set_common_prefix(
        &mut self,
        list: &[std::borrow::Cow<'_, str>],
    ) -> Result<(), PrefixListDecodeError> {
        self.prefixes = list
            .iter()
            .map(|item| decode_oss_encoded_field(item.as_ref()))
            .map(normalize_prefix)
            .filter(|item| !item.is_empty())
            .collect();
        Ok(())
    }

    fn set_next_continuation_token_str(
        &mut self,
        token: &str,
    ) -> Result<(), PrefixListDecodeError> {
        self.next_continuation_token = token.to_string();
        Ok(())
    }
}

impl InitObject<PrefixFallbackItem> for PrefixFallbackList {
    fn init_object(&mut self) -> Option<PrefixFallbackItem> {
        Some(PrefixFallbackItem)
    }
}

impl RefineObject<PrefixListDecodeError> for ObjectKeyFallbackItem {
    fn set_key(&mut self, key: &str) -> Result<(), PrefixListDecodeError> {
        self.key = decode_oss_encoded_field(key);
        Ok(())
    }
}

impl RefineObjectList<ObjectKeyFallbackItem, PrefixListDecodeError> for ObjectKeyFallbackList {
    fn set_list(&mut self, list: Vec<ObjectKeyFallbackItem>) -> Result<(), PrefixListDecodeError> {
        self.keys = list
            .into_iter()
            .map(|item| item.key)
            .filter(|item| !item.is_empty())
            .collect();
        Ok(())
    }

    fn set_next_continuation_token_str(
        &mut self,
        token: &str,
    ) -> Result<(), PrefixListDecodeError> {
        self.next_continuation_token = token.to_string();
        Ok(())
    }
}

impl InitObject<ObjectKeyFallbackItem> for ObjectKeyFallbackList {
    fn init_object(&mut self) -> Option<ObjectKeyFallbackItem> {
        Some(ObjectKeyFallbackItem::default())
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BucketsResponse {
    buckets: Vec<BucketSummary>,
    current_bucket: String,
    current_region: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BucketPreviewPayload {
    access_key_id: String,
    access_key_secret: String,
    region: Option<String>,
}

fn normalize_prefix(value: impl AsRef<str>) -> String {
    let normalized = value
        .as_ref()
        .split('/')
        .map(str::trim)
        .filter(|segment| !segment.is_empty() && *segment != "." && *segment != "..")
        .collect::<Vec<_>>()
        .join("/");

    if normalized.is_empty() {
        String::new()
    } else {
        format!("{normalized}/")
    }
}

fn decode_oss_encoded_field(value: &str) -> String {
    if value.is_empty() {
        return String::new();
    }

    urlencoding::decode(value)
        .map(|decoded| decoded.into_owned())
        .unwrap_or_else(|_| value.to_string())
}

fn validate_upload_path(value: impl AsRef<str>) -> Result<String, anyhow::Error> {
    let raw = value.as_ref().trim().to_string();

    if raw.is_empty() {
        return Ok(String::new());
    }

    if raw.starts_with('/') {
        return Err(anyhow!("上传路径不能以 / 开头"));
    }

    if !raw.ends_with('/') {
        return Err(anyhow!("上传路径必须以 / 结尾"));
    }

    if raw.contains('\\') {
        return Err(anyhow!("上传路径不能包含反斜杠 \\"));
    }

    if raw.contains("//") {
        return Err(anyhow!("上传路径不能包含连续的 /"));
    }

    let segments = raw.split('/').filter(|segment| !segment.is_empty());
    for segment in segments {
        if segment == "." || segment == ".." {
            return Err(anyhow!("上传路径不能包含 . 或 .."));
        }

        if segment.trim() != segment {
            return Err(anyhow!("上传路径包含无效目录名"));
        }
    }

    Ok(raw)
}

fn normalize_region(value: impl AsRef<str>) -> String {
    let region = value.as_ref().trim();
    if region.is_empty() {
        DEFAULT_REGION.to_string()
    } else if region.starts_with("oss-") {
        region.to_string()
    } else {
        format!("oss-{region}")
    }
}

fn parse_endpoint(region: &str) -> Result<EndPoint, anyhow::Error> {
    let normalized = normalize_region(region);
    let endpoint_value = normalized
        .strip_prefix("oss-")
        .unwrap_or(normalized.as_str());

    endpoint_value
        .parse()
        .with_context(|| format!("无效的 OSS Region: {normalized}"))
}

fn normalize_config(config: LocalConfig) -> LocalConfig {
    LocalConfig {
        access_key_id: config.access_key_id.trim().to_string(),
        access_key_secret: config.access_key_secret.trim().to_string(),
        bucket: config.bucket.trim().to_string(),
        region: normalize_region(config.region),
        prefix: normalize_prefix(config.prefix),
    }
}

fn config_path(app: &AppHandle) -> Result<PathBuf, anyhow::Error> {
    let dir = if cfg!(target_os = "macos") {
        match std::env::var_os("HOME") {
            Some(home) => PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("oss-uploader"),
            None => {
                return Err(anyhow!("无法定位当前用户目录"));
            }
        }
    } else {
        app.path()
            .app_config_dir()
            .map_err(|err| anyhow!(err.to_string()))?
    };

    fs::create_dir_all(&dir).context("创建配置目录失败")?;
    Ok(dir.join("oss-uploader-config.json"))
}

fn read_local_config(app: &AppHandle) -> Result<LocalConfig, anyhow::Error> {
    let file_path = config_path(app)?;
    match fs::read_to_string(&file_path) {
        Ok(content) => {
            let config: LocalConfig = serde_json::from_str(&content).context("解析本地配置失败")?;
            Ok(normalize_config(config))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(normalize_config(LocalConfig {
            region: DEFAULT_REGION.to_string(),
            ..Default::default()
        })),
        Err(err) => Err(err).context("读取本地配置失败"),
    }
}

fn has_oss_config(config: &LocalConfig) -> bool {
    !config.access_key_id.is_empty() && !config.access_key_secret.is_empty()
}

fn has_bucket_target(config: &LocalConfig) -> bool {
    has_oss_config(config) && !config.bucket.is_empty()
}

fn public_config(app: &AppHandle) -> Result<PublicConfig, anyhow::Error> {
    let config = read_local_config(app)?;
    let has_config = has_oss_config(&config);
    Ok(PublicConfig {
        bucket: config.bucket,
        region: config.region,
        prefix: config.prefix,
        has_oss_config: has_config,
        config_path: config_path(app)?.display().to_string(),
    })
}

fn build_client(config: &LocalConfig) -> Result<Client, anyhow::Error> {
    let endpoint = parse_endpoint(&config.region)?;
    let bucket = BucketName::new(config.bucket.clone()).context("无效的 Bucket 名称")?;

    Ok(Client::new(
        KeyId::new(config.access_key_id.clone()),
        KeySecret::new(config.access_key_secret.clone()),
        endpoint,
        bucket,
    ))
}

fn build_bucket_list_client(
    access_key_id: String,
    access_key_secret: String,
    region: String,
) -> Result<Client, anyhow::Error> {
    let endpoint = parse_endpoint(&region)?;
    let bucket = BucketName::new("bucket-placeholder".to_string()).context("无效的 Bucket 名称")?;

    Ok(Client::new(
        KeyId::new(access_key_id),
        KeySecret::new(access_key_secret),
        endpoint,
        bucket,
    ))
}

async fn validate_oss_access(config: &LocalConfig) -> Result<(), anyhow::Error> {
    let client = build_client(config)?;
    let query = vec![
        ("delimiter".into(), "/".into()),
        ("max-keys".into(), 1u16.into()),
        ("encoding-type".into(), "url".into()),
    ];
    let mut list = PrefixFallbackList::default();

    if client.base_object_list(query, &mut list).await.is_ok() {
        return Ok(());
    }

    // 部分 Bucket 会因历史对象键触发 XML 解码异常，回退到 Bucket 列表校验避免误判不可用。
    let bucket_list_client = build_bucket_list_client(
        config.access_key_id.clone(),
        config.access_key_secret.clone(),
        config.region.clone(),
    )?;

    let buckets = list_bucket_summaries(bucket_list_client).await?;
    if buckets.iter().any(|bucket| bucket.name == config.bucket) {
        return Ok(());
    }

    Err(anyhow!("无法访问 Bucket {}", config.bucket))
}

fn object_url(config: &LocalConfig, key: &str) -> String {
    let encoded_key = key
        .split('/')
        .map(urlencoding::encode)
        .map(|segment| segment.into_owned())
        .collect::<Vec<_>>()
        .join("/");

    format!(
        "https://{}.{}.aliyuncs.com/{}",
        config.bucket, config.region, encoded_key
    )
}

async fn list_bucket_summaries(client: Client) -> Result<Vec<BucketSummary>, anyhow::Error> {
    let mut bucket_list = BucketSummaryList::default();

    client
        .base_bucket_list(&mut bucket_list)
        .await
        .map_err(|err| {
            let detail = std::error::Error::source(&err)
                .map(|source| source.to_string())
                .unwrap_or_else(|| err.to_string());
            anyhow!("读取 Bucket 列表失败: {detail}")
        })?;

    Ok(bucket_list
        .buckets
        .into_iter()
        .map(|bucket| BucketSummary {
            name: bucket.name,
            region: bucket.region,
        })
        .collect())
}

async fn list_child_prefixes_primary(
    client: &Client,
    parent_prefix: &str,
) -> Result<Vec<String>, anyhow::Error> {
    let mut continuation_token: Option<String> = None;
    let mut prefixes = Vec::<String>::new();

    loop {
        let mut query = vec![
            ("delimiter".into(), "/".into()),
            ("max-keys".into(), 1000u16.into()),
            ("encoding-type".into(), "url".into()),
        ];

        if !parent_prefix.is_empty() {
            query.push(("prefix".into(), parent_prefix.to_string().into()));
        }

        if let Some(token) = continuation_token.as_ref().filter(|token| !token.is_empty()) {
            query.push(("continuation-token".into(), token.to_string().into()));
        }

        let list = client.get_object_list(query).await.map_err(|err| {
            let detail = std::error::Error::source(&err)
                .map(|source| source.to_string())
                .unwrap_or_else(|| err.to_string());
            anyhow!("读取路径失败: {detail}")
        })?;

        prefixes.extend(
            list.common_prefixes()
                .iter()
                .map(|item| decode_oss_encoded_field(item.as_ref()))
                .map(normalize_prefix)
                .filter(|item| !item.is_empty()),
        );

        let next_token = list.next_continuation_token_str().trim();
        if next_token.is_empty() {
            break;
        }
        continuation_token = Some(next_token.to_string());
    }

    prefixes.sort();
    prefixes.dedup();
    Ok(prefixes)
}

async fn list_child_prefixes_fallback(
    client: &Client,
    parent_prefix: &str,
) -> Result<Vec<String>, anyhow::Error> {
    let mut continuation_token: Option<String> = None;
    let mut prefixes = Vec::<String>::new();

    loop {
        let mut query = vec![
            ("delimiter".into(), "/".into()),
            ("max-keys".into(), 1000u16.into()),
            ("encoding-type".into(), "url".into()),
        ];

        if !parent_prefix.is_empty() {
            query.push(("prefix".into(), parent_prefix.to_string().into()));
        }

        if let Some(token) = continuation_token.as_ref().filter(|token| !token.is_empty()) {
            query.push(("continuation-token".into(), token.to_string().into()));
        }

        let mut list = PrefixFallbackList::default();
        client
            .base_object_list(query, &mut list)
            .await
            .map_err(|err| {
                let detail = std::error::Error::source(&err)
                    .map(|source| source.to_string())
                    .unwrap_or_else(|| err.to_string());
                anyhow!("备用通道读取路径失败: {detail}")
            })?;

        prefixes.extend(list.prefixes.into_iter());

        let next_token = list.next_continuation_token.trim();
        if next_token.is_empty() {
            break;
        }
        continuation_token = Some(next_token.to_string());
    }

    prefixes.sort();
    prefixes.dedup();
    Ok(prefixes)
}

async fn list_child_prefixes_with_fallback(
    client: &Client,
    parent_prefix: &str,
) -> Result<(Vec<String>, bool), anyhow::Error> {
    match list_child_prefixes_primary(client, parent_prefix).await {
        Ok(prefixes) => Ok((prefixes, false)),
        Err(primary_err) => match list_child_prefixes_fallback(client, parent_prefix).await {
            Ok(prefixes) => Ok((prefixes, true)),
            Err(fallback_err) => Err(anyhow!(
                "主通道失败: {}; 备用通道失败: {}",
                primary_err,
                fallback_err
            )),
        },
    }
}

async fn list_all_prefixes_with_fallback(
    client: &Client,
) -> Result<(Vec<String>, AllPrefixesStats), anyhow::Error> {
    // 快速通道：按对象分页一次性扫描并提取前缀，通常请求数最少、速度最高。
    let fast_scan_error = match list_prefixes_by_object_scan(client, "").await {
        Ok(scan) => {
            return Ok((
                scan.prefixes,
                AllPrefixesStats {
                    visited: scan.visited_pages,
                    fallback_hits: scan.fallback_hits,
                    failed_parents: vec![],
                    truncated: scan.truncated,
                },
            ));
        }
        Err(err) => Some(err.to_string()),
    };

    // 容错通道：目录前缀 BFS + 分支对象扫描兜底。
    let mut discovered = HashSet::<String>::new();
    let mut queued = HashSet::<String>::new();
    let mut queue = VecDeque::<String>::new();
    let mut failed_parents = Vec::<String>::new();
    let mut fallback_hits = 0usize;
    let mut truncated = false;
    let mut visited_parents = 0usize;

    queue.push_back(String::new());
    queued.insert(String::new());

    while let Some(parent_prefix) = queue.pop_front() {
        if visited_parents >= MAX_PREFIX_SCAN_PARENTS || discovered.len() >= MAX_PREFIX_TOTAL_ITEMS {
            truncated = true;
            break;
        }
        visited_parents += 1;

        match list_child_prefixes_with_fallback(client, &parent_prefix).await {
            Ok((children, used_fallback)) => {
                if used_fallback {
                    fallback_hits += 1;
                }

                for child in children {
                    let normalized = normalize_prefix(child);
                    if normalized.is_empty() {
                        continue;
                    }

                    if discovered.insert(normalized.clone()) {
                        if queued.insert(normalized.clone()) {
                            queue.push_back(normalized);
                        }
                    }

                    if discovered.len() >= MAX_PREFIX_TOTAL_ITEMS {
                        truncated = true;
                        break;
                    }
                }
            }

            Err(primary_and_fallback_err) => {
                match list_prefixes_by_object_scan(client, &parent_prefix).await {
                    Ok(extra_scan) => {
                        fallback_hits += 1 + extra_scan.fallback_hits;
                        for prefix in extra_scan.prefixes {
                            let normalized = normalize_prefix(prefix);
                            if normalized.is_empty() {
                                continue;
                            }

                            discovered.insert(normalized);
                            if discovered.len() >= MAX_PREFIX_TOTAL_ITEMS {
                                truncated = true;
                                break;
                            }
                        }
                    }
                    Err(scan_err) => {
                        let marker = if parent_prefix.is_empty() {
                            "<root>".to_string()
                        } else {
                            parent_prefix.clone()
                        };
                        failed_parents.push(format!(
                            "{} (目录枚举失败: {}; 对象扫描失败: {})",
                            marker, primary_and_fallback_err, scan_err
                        ));
                    }
                }
            }
        };

        if truncated {
            break;
        }
    }

    let mut prefixes = discovered.into_iter().collect::<Vec<_>>();
    prefixes.sort();
    prefixes.dedup();

    if let Some(message) = fast_scan_error {
        failed_parents.insert(0, format!("<root> (快速扫描失败: {message})"));
    }

    Ok((
        prefixes,
        AllPrefixesStats {
            visited: visited_parents,
            fallback_hits,
            failed_parents,
            truncated,
        },
    ))
}

struct ObjectScanResult {
    prefixes: Vec<String>,
    visited_pages: usize,
    fallback_hits: usize,
    truncated: bool,
}

async fn list_prefixes_by_object_scan(
    client: &Client,
    parent_prefix: &str,
) -> Result<ObjectScanResult, anyhow::Error> {
    let mut discovered = HashSet::<String>::new();
    let mut continuation_token: Option<String> = None;
    let mut visited_pages = 0usize;
    let mut scanned_keys = 0usize;
    let mut fallback_hits = 0usize;
    let mut truncated = false;

    loop {
        if visited_pages >= MAX_OBJECT_SCAN_PAGES
            || scanned_keys >= MAX_OBJECT_SCAN_KEYS
            || discovered.len() >= MAX_PREFIX_TOTAL_ITEMS
        {
            truncated = true;
            break;
        }
        visited_pages += 1;

        let page = list_object_page_primary(client, continuation_token.as_deref(), Some(parent_prefix)).await;
        let (keys, next_token) = match page {
            Ok(result) => result,
            Err(primary_err) => {
                match list_object_page_fallback(client, continuation_token.as_deref(), Some(parent_prefix)).await {
                    Ok(result) => {
                        fallback_hits += 1;
                        result
                    }
                    Err(fallback_err) => {
                        return Err(anyhow!(
                            "主通道失败: {}; 备用通道失败: {}",
                            primary_err,
                            fallback_err
                        ));
                    }
                }
            }
        };

        scanned_keys += keys.len();
        for key in keys {
            if !parent_prefix.is_empty() && !key.starts_with(parent_prefix) {
                continue;
            }
            collect_prefixes_from_key(&key, &mut discovered);
            if discovered.len() >= MAX_PREFIX_TOTAL_ITEMS {
                break;
            }
        }

        if next_token.is_empty() {
            break;
        }
        continuation_token = Some(next_token);
    }

    let mut prefixes = discovered.into_iter().collect::<Vec<_>>();
    prefixes.sort();
    prefixes.dedup();
    Ok(ObjectScanResult {
        prefixes,
        visited_pages,
        fallback_hits,
        truncated,
    })
}

fn collect_prefixes_from_key(key: &str, discovered: &mut HashSet<String>) {
    let normalized_key = key.trim();
    if normalized_key.is_empty() {
        return;
    }

    let segments = normalized_key
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();

    if segments.is_empty() {
        return;
    }

    let include_depth = if normalized_key.ends_with('/') {
        segments.len()
    } else {
        segments.len().saturating_sub(1)
    };

    for depth in 1..=include_depth {
        let prefix = format!("{}/", segments[..depth].join("/"));
        if !prefix.is_empty() {
            discovered.insert(prefix);
        }
    }
}

async fn list_object_page_primary(
    client: &Client,
    continuation_token: Option<&str>,
    prefix: Option<&str>,
) -> Result<(Vec<String>, String), anyhow::Error> {
    let mut query = vec![
        ("max-keys".into(), 1000u16.into()),
        ("encoding-type".into(), "url".into()),
    ];

    if let Some(path_prefix) = prefix.filter(|item| !item.is_empty()) {
        query.push(("prefix".into(), path_prefix.to_string().into()));
    }

    if let Some(token) = continuation_token.filter(|token| !token.is_empty()) {
        query.push(("continuation-token".into(), token.to_string().into()));
    }

    let list = client.get_object_list(query).await.map_err(|err| {
        let detail = std::error::Error::source(&err)
            .map(|source| source.to_string())
            .unwrap_or_else(|| err.to_string());
        anyhow!("读取对象列表失败: {detail}")
    })?;

    let next_token = list.next_continuation_token_str().trim().to_string();
    let keys = list
        .object_iter()
        .map(|object| decode_oss_encoded_field(&object.path_string()))
        .filter(|item| !item.is_empty())
        .collect::<Vec<_>>();

    Ok((keys, next_token))
}

async fn list_object_page_fallback(
    client: &Client,
    continuation_token: Option<&str>,
    prefix: Option<&str>,
) -> Result<(Vec<String>, String), anyhow::Error> {
    let mut query = vec![
        ("max-keys".into(), 1000u16.into()),
        ("encoding-type".into(), "url".into()),
    ];

    if let Some(path_prefix) = prefix.filter(|item| !item.is_empty()) {
        query.push(("prefix".into(), path_prefix.to_string().into()));
    }

    if let Some(token) = continuation_token.filter(|token| !token.is_empty()) {
        query.push(("continuation-token".into(), token.to_string().into()));
    }

    let mut list = ObjectKeyFallbackList::default();
    client
        .base_object_list(query, &mut list)
        .await
        .map_err(|err| {
            let detail = std::error::Error::source(&err)
                .map(|source| source.to_string())
                .unwrap_or_else(|| err.to_string());
            anyhow!("备用通道读取对象列表失败: {detail}")
        })?;

    Ok((list.keys, list.next_continuation_token))
}

fn normalize_bucket_region(value: impl AsRef<str>) -> String {
    let region = value.as_ref().trim();
    if region.is_empty() {
        DEFAULT_REGION.to_string()
    } else if region.starts_with("oss-") {
        region.to_string()
    } else {
        format!("oss-{region}")
    }
}

#[tauri::command]
fn get_config(app: AppHandle) -> Result<PublicConfig, String> {
    public_config(&app).map_err(|err| err.to_string())
}

#[tauri::command]
fn reset_config(app: AppHandle) -> Result<SaveConfigResponse, String> {
    let reset = normalize_config(LocalConfig {
        access_key_id: String::new(),
        access_key_secret: String::new(),
        bucket: String::new(),
        region: DEFAULT_REGION.to_string(),
        prefix: String::new(),
    });

    let file_path = config_path(&app).map_err(|err| err.to_string())?;
    let content = format!(
        "{}\n",
        serde_json::to_string_pretty(&reset).map_err(|err| err.to_string())?
    );
    fs::write(file_path, content).map_err(|err| err.to_string())?;

    Ok(SaveConfigResponse {
        ok: true,
        config: public_config(&app).map_err(|err| err.to_string())?,
    })
}

#[tauri::command]
async fn list_buckets_preview(payload: BucketPreviewPayload) -> Result<Vec<BucketSummary>, String> {
    let access_key_id = payload.access_key_id.trim().to_string();
    let access_key_secret = payload.access_key_secret.trim().to_string();
    let region = normalize_region(payload.region.unwrap_or_default());

    if access_key_id.is_empty() || access_key_secret.is_empty() {
        return Err("请先填写 Access Key ID 和 Access Key Secret".to_string());
    }

    let client = build_bucket_list_client(access_key_id, access_key_secret, region)
        .map_err(|err| err.to_string())?;

    list_bucket_summaries(client)
        .await
        .map_err(|err| err.to_string())
}

#[tauri::command]
async fn list_buckets(app: AppHandle) -> Result<BucketsResponse, String> {
    let config = read_local_config(&app).map_err(|err| err.to_string())?;
    if !has_oss_config(&config) {
        return Err("请先填写 Access Key ID 和 Access Key Secret".to_string());
    }

    let client = build_bucket_list_client(
        config.access_key_id.clone(),
        config.access_key_secret.clone(),
        config.region.clone(),
    )
    .map_err(|err| err.to_string())?;
    let buckets = list_bucket_summaries(client)
        .await
        .map_err(|err| err.to_string())?;

    Ok(BucketsResponse {
        buckets,
        current_bucket: config.bucket,
        current_region: config.region,
    })
}

#[tauri::command]
async fn save_config(app: AppHandle, payload: SaveConfigPayload) -> Result<SaveConfigResponse, String> {
    let current = read_local_config(&app).map_err(|err| err.to_string())?;
    let current_snapshot = current.clone();
    let next = normalize_config(LocalConfig {
        access_key_id: payload.access_key_id.unwrap_or(current.access_key_id),
        access_key_secret: payload
            .access_key_secret
            .unwrap_or(current.access_key_secret),
        bucket: payload.bucket.unwrap_or(current.bucket),
        region: payload.region.unwrap_or(current.region),
        prefix: payload.prefix.unwrap_or(current.prefix),
    });

    if !has_oss_config(&next) {
        return Err("请填写 Access Key ID 和 Access Key Secret".to_string());
    }

    // prefix-only 更新无需重复校验 OSS 可达性，避免每次切路径都触发额外网络开销。
    let should_validate_access = next.access_key_id != current_snapshot.access_key_id
        || next.access_key_secret != current_snapshot.access_key_secret
        || next.bucket != current_snapshot.bucket
        || next.region != current_snapshot.region;

    if should_validate_access && !next.bucket.is_empty() {
        validate_oss_access(&next).await.map_err(|err| err.to_string())?;
    }

    let file_path = config_path(&app).map_err(|err| err.to_string())?;
    let content = format!(
        "{}\n",
        serde_json::to_string_pretty(&next).map_err(|err| err.to_string())?
    );
    fs::write(file_path, content).map_err(|err| err.to_string())?;

    Ok(SaveConfigResponse {
        ok: true,
        config: public_config(&app).map_err(|err| err.to_string())?,
    })
}

#[tauri::command]
async fn list_prefixes(app: AppHandle, parent: Option<String>) -> Result<PrefixesResponse, String> {
    let config = read_local_config(&app).map_err(|err| err.to_string())?;
    if !has_bucket_target(&config) {
        return Err("请先在主页面选择 Bucket".to_string());
    }

    let client = build_client(&config).map_err(|err| err.to_string())?;
    let parent_prefix = normalize_prefix(parent.unwrap_or_default());
    let prefixes = list_child_prefixes_with_fallback(&client, &parent_prefix)
        .await
        .map(|(prefixes, _)| prefixes)
        .map_err(|err| err.to_string())?;

    Ok(PrefixesResponse {
        prefixes,
        parent_prefix,
        default_prefix: config.prefix,
    })
}

#[tauri::command]
async fn list_all_prefixes(app: AppHandle) -> Result<AllPrefixesResponse, String> {
    let config = read_local_config(&app).map_err(|err| err.to_string())?;
    if !has_bucket_target(&config) {
        return Err("请先在主页面选择 Bucket".to_string());
    }

    let client = build_client(&config).map_err(|err| err.to_string())?;
    let (prefixes, stats) = list_all_prefixes_with_fallback(&client)
        .await
        .map_err(|err| err.to_string())?;

    Ok(AllPrefixesResponse {
        prefixes,
        default_prefix: config.prefix,
        stats,
    })
}

#[tauri::command]
async fn upload_files(
    app: AppHandle,
    path: Option<String>,
    files: Vec<UploadFilePayload>,
) -> Result<UploadResponse, String> {
    let config = read_local_config(&app).map_err(|err| err.to_string())?;
    if !has_bucket_target(&config) {
        return Err("请先在主页面选择 Bucket".to_string());
    }

    let client = build_client(&config).map_err(|err| err.to_string())?;
    let target_prefix = validate_upload_path(path.unwrap_or(config.prefix.clone()))
        .map_err(|err| err.to_string())?;
    let mut results = Vec::with_capacity(files.len());

    for file in files {
        let key = format!("{}{}", target_prefix, file.name);
        let mime_type = if file.mime_type.trim().is_empty() {
            "application/octet-stream".to_string()
        } else {
            file.mime_type.clone()
        };
        let upload_key = key.clone();

        match client
            .put_content_base(file.bytes, &mime_type, upload_key)
            .await
        {
            Ok(_) => results.push(UploadItem {
                name: file.name,
                key: key.clone(),
                prefix: target_prefix.clone(),
                url: Some(object_url(&config, &key)),
                size: file.size,
                ok: true,
                error: None,
            }),
            Err(err) => results.push(UploadItem {
                name: file.name,
                key,
                prefix: target_prefix.clone(),
                url: None,
                size: file.size,
                ok: false,
                error: Some(err.to_string()),
            }),
        }
    }

    Ok(UploadResponse { results })
}

#[tauri::command]
async fn upload_file_paths(
    app: AppHandle,
    path: Option<String>,
    files: Vec<UploadLocalPathPayload>,
) -> Result<UploadResponse, String> {
    let config = read_local_config(&app).map_err(|err| err.to_string())?;
    if !has_bucket_target(&config) {
        return Err("请先在主页面选择 Bucket".to_string());
    }

    let client = build_client(&config).map_err(|err| err.to_string())?;
    let target_prefix = validate_upload_path(path.unwrap_or(config.prefix.clone()))
        .map_err(|err| err.to_string())?;
    let mut results = Vec::with_capacity(files.len());

    for file in files {
        let local_path = file.path.trim().to_string();
        let source_path = PathBuf::from(local_path.clone());
        let fallback_name = source_path
            .file_name()
            .and_then(|item| item.to_str())
            .map(|item| item.to_string())
            .unwrap_or_else(|| "unnamed".to_string());
        let name = file
            .name
            .unwrap_or(fallback_name)
            .trim()
            .to_string();

        if name.is_empty() {
            results.push(UploadItem {
                name: "unnamed".to_string(),
                key: String::new(),
                prefix: target_prefix.clone(),
                url: None,
                size: 0,
                ok: false,
                error: Some("本地文件名无效".to_string()),
            });
            continue;
        }

        let key = format!("{}{}", target_prefix, name);

        let bytes = match fs::read(&source_path) {
            Ok(bytes) => bytes,
            Err(err) => {
                results.push(UploadItem {
                    name,
                    key,
                    prefix: target_prefix.clone(),
                    url: None,
                    size: 0,
                    ok: false,
                    error: Some(format!("读取本地文件失败: {err}")),
                });
                continue;
            }
        };

        let file_size = bytes.len() as u64;
        let upload_key = key.clone();

        match client
            .put_content_base(bytes, "application/octet-stream", upload_key)
            .await
        {
            Ok(_) => results.push(UploadItem {
                name,
                key: key.clone(),
                prefix: target_prefix.clone(),
                url: Some(object_url(&config, &key)),
                size: file_size,
                ok: true,
                error: None,
            }),
            Err(err) => results.push(UploadItem {
                name,
                key,
                prefix: target_prefix.clone(),
                url: None,
                size: file_size,
                ok: false,
                error: Some(err.to_string()),
            }),
        }
    }

    Ok(UploadResponse { results })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_log::Builder::default().level(log::LevelFilter::Info).build())
        .invoke_handler(tauri::generate_handler![
            get_config,
            reset_config,
            list_buckets_preview,
            list_buckets,
            save_config,
            list_prefixes,
            list_all_prefixes,
            upload_files,
            upload_file_paths
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
