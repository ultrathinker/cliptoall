use aws_config::BehaviorVersion;
use aws_sdk_s3::primitives::ByteStream;
use std::path::Path;

#[tauri::command]
pub async fn upload_to_s3(
    image_path: String,
    output_scale: f32,
) -> Result<String, String> {
    let settings = crate::commands::settings::load_settings_sync();
    let access_key = settings.amazon_access_key_id;
    let secret_key = settings.amazon_secret_access_key;
    let bucket = settings.amazon_bucket;
    let region = settings.amazon_region;
    let folder = settings.amazon_s3folder;

    if access_key.is_empty() || secret_key.is_empty() {
        return Err("S3 storage is not configured. Go to Settings and enter your S3 credentials.".to_string());
    }

    // Working copies are full-res lossless PNG — transcode to JPEG (and apply the
    // DPI downscale for output if enabled) once here, at the configured quality.
    let image_path = super::capture::ensure_jpeg_for_upload(&image_path, output_scale)?;

    let folder = if folder.is_empty() { "d".to_string() } else { folder };

    let credentials = aws_sdk_s3::config::Credentials::new(
        access_key,
        secret_key,
        None,
        None,
        "cliptoall",
    );

    let config = aws_config::defaults(BehaviorVersion::latest())
        .credentials_provider(credentials)
        .region(aws_config::Region::new(region.clone()))
        .load()
        .await;

    let client = aws_sdk_s3::Client::new(&config);

    let path = Path::new(&image_path);
    let filename = path.file_name().unwrap().to_string_lossy();

    let now = chrono::Local::now();
    let date_folder = now.format("%Y%m%d").to_string();
    let key = format!("{}/{}/{}", folder, date_folder, filename);

    let body = ByteStream::from_path(&path)
        .await
        .map_err(|e| format!("Failed to read file: {}", e))?;

    // Upload without ACL — bucket policy handles public access.
    // Bounded by a hard timeout so a stalled connection can't spin forever (BUGS#4a).
    let send_fut = client
        .put_object()
        .bucket(&bucket)
        .key(&key)
        .body(body)
        .content_type("image/jpeg")
        .send();
    match tokio::time::timeout(std::time::Duration::from_secs(60), send_fut).await {
        Ok(res) => { res.map_err(|e| format!("Failed to upload: {:?}", e))?; }
        Err(_) => return Err("S3 upload timed out after 60s".to_string()),
    }

    let encoded_key = encode_s3_key_for_url(&key);
    let url = format!("https://{}.s3.{}.amazonaws.com/{}", bucket, region, encoded_key);
    Ok(url)
}

fn encode_s3_key_for_url(key: &str) -> String {
    key.split('/')
        .map(|segment| urlencoding::encode(segment).into_owned())
        .collect::<Vec<_>>()
        .join("/")
}
