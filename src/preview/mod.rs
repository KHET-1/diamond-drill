//! Preview module - Thumbnail generation and file previews
//!
//! Provides progressive thumbnail generation with turbojpeg optimization.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use image::imageops::FilterType;
use image::{DynamicImage, ImageFormat};
use parking_lot::RwLock;
use rayon::prelude::*;

/// Cache for generated thumbnails
type ThumbnailCache = Arc<RwLock<std::collections::HashMap<String, PathBuf>>>;

/// Thumbnail generator with progressive loading
pub struct ThumbnailGenerator {
    /// Cache directory for thumbnails
    cache_dir: PathBuf,
    /// In-memory cache of thumbnail paths
    cache: ThumbnailCache,
}

impl ThumbnailGenerator {
    /// Create a new thumbnail generator
    pub fn new() -> Self {
        let cache_dir = directories::ProjectDirs::from("com", "tunclon", "diamond-drill")
            .map(|dirs| dirs.cache_dir().join("thumbnails"))
            .unwrap_or_else(|| PathBuf::from(".diamond-drill-cache/thumbnails"));

        // Ensure cache directory exists
        std::fs::create_dir_all(&cache_dir).ok();

        Self {
            cache_dir,
            cache: Arc::new(RwLock::new(std::collections::HashMap::new())),
        }
    }

    /// Generate progressive thumbnails (small first, then larger)
    ///
    /// Returns the path to the final thumbnail.
    pub fn generate_progressive(
        &self,
        source: &Path,
        small_size: u32,
        large_size: u32,
    ) -> Result<PathBuf> {
        let cache_key = self.cache_key(source, large_size);

        // Check cache first
        if let Some(cached) = self.cache.read().get(&cache_key) {
            if cached.exists() {
                return Ok(cached.clone());
            }
        }

        // Load image
        let img = image::open(source)
            .with_context(|| format!("Failed to open image: {}", source.display()))?;

        // Generate small thumbnail first (64x64) - fast preview
        let small_thumb = self.resize_image(&img, small_size);
        let small_path = self.thumbnail_path(source, small_size);
        self.save_thumbnail(&small_thumb, &small_path)?;

        // Generate larger thumbnail (512x512) - detailed preview
        let large_thumb = self.resize_image(&img, large_size);
        let large_path = self.thumbnail_path(source, large_size);
        self.save_thumbnail(&large_thumb, &large_path)?;

        // Cache the result
        self.cache.write().insert(cache_key, large_path.clone());

        Ok(large_path)
    }

    /// Generate a single thumbnail at specified size
    pub fn generate(&self, source: &Path, size: u32) -> Result<PathBuf> {
        let cache_key = self.cache_key(source, size);

        // Check cache
        if let Some(cached) = self.cache.read().get(&cache_key) {
            if cached.exists() {
                return Ok(cached.clone());
            }
        }

        // Load and resize
        let img = image::open(source)
            .with_context(|| format!("Failed to open image: {}", source.display()))?;

        let thumb = self.resize_image(&img, size);
        let thumb_path = self.thumbnail_path(source, size);
        self.save_thumbnail(&thumb, &thumb_path)?;

        // Cache
        self.cache.write().insert(cache_key, thumb_path.clone());

        Ok(thumb_path)
    }

    /// Resize image maintaining aspect ratio
    fn resize_image(&self, img: &DynamicImage, max_size: u32) -> DynamicImage {
        let (width, height) = (img.width(), img.height());

        // Calculate new dimensions maintaining aspect ratio
        let (new_width, new_height) = if width > height {
            let ratio = max_size as f32 / width as f32;
            (max_size, (height as f32 * ratio) as u32)
        } else {
            let ratio = max_size as f32 / height as f32;
            ((width as f32 * ratio) as u32, max_size)
        };

        // Use Lanczos3 for best quality, but can use Triangle for speed
        img.resize(new_width, new_height, FilterType::Lanczos3)
    }

    /// Save thumbnail to disk
    fn save_thumbnail(&self, img: &DynamicImage, path: &Path) -> Result<()> {
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Save as JPEG with quality 85 (good balance of size/quality)
        let mut output = std::fs::File::create(path)?;
        img.write_to(&mut output, ImageFormat::Jpeg)?;

        Ok(())
    }

    /// Generate cache key for a source path and size
    fn cache_key(&self, source: &Path, size: u32) -> String {
        let hash = blake3::hash(source.to_string_lossy().as_bytes());
        format!("{}-{}", hex::encode(&hash.as_bytes()[..8]), size)
    }

    /// Get thumbnail path for source and size
    fn thumbnail_path(&self, source: &Path, size: u32) -> PathBuf {
        let key = self.cache_key(source, size);
        self.cache_dir.join(format!("{}.jpg", key))
    }

    /// Get cache directory
    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    /// Clear thumbnail cache
    pub fn clear_cache(&self) -> Result<()> {
        self.cache.write().clear();
        if self.cache_dir.exists() {
            std::fs::remove_dir_all(&self.cache_dir)?;
            std::fs::create_dir_all(&self.cache_dir)?;
        }
        Ok(())
    }

    /// Generate progressive thumbnails at multiple sizes in one pass
    ///
    /// Loads the image once and creates all requested sizes, applying EXIF rotation.
    /// Returns paths for each requested size.
    pub fn generate_progressive_multi(&self, source: &Path, sizes: &[u32]) -> Result<Vec<PathBuf>> {
        if sizes.is_empty() {
            return Ok(Vec::new());
        }

        // Load image once
        let img = image::open(source)
            .with_context(|| format!("Failed to open image: {}", source.display()))?;

        // Apply EXIF rotation
        let img = self.apply_exif_rotation(source, img);

        let mut paths = Vec::with_capacity(sizes.len());

        // Sort sizes ascending so we can generate small first (faster preview)
        let mut sorted_sizes = sizes.to_vec();
        sorted_sizes.sort_unstable();

        for &size in &sorted_sizes {
            let cache_key = self.cache_key(source, size);

            // Check cache
            if let Some(cached) = self.cache.read().get(&cache_key) {
                if cached.exists() {
                    paths.push(cached.clone());
                    continue;
                }
            }

            let thumb = self.resize_image(&img, size);
            let thumb_path = self.thumbnail_path(source, size);
            self.save_thumbnail(&thumb, &thumb_path)?;

            self.cache.write().insert(cache_key, thumb_path.clone());
            paths.push(thumb_path);
        }

        Ok(paths)
    }

    /// Generate thumbnails for a batch of files in parallel using rayon
    ///
    /// Returns a Vec of Results, one per input path.
    pub fn generate_batch(&self, sources: &[PathBuf], size: u32) -> Vec<Result<PathBuf>> {
        sources
            .par_iter()
            .map(|source| self.generate(source, size))
            .collect()
    }

    /// Read EXIF orientation and apply rotation/flip to correct image orientation
    fn apply_exif_rotation(&self, source: &Path, img: DynamicImage) -> DynamicImage {
        let orientation = read_exif_orientation(source);
        match orientation {
            // 1 = normal, no rotation needed
            1 => img,
            // 2 = flipped horizontally
            2 => img.fliph(),
            // 3 = rotated 180
            3 => img.rotate180(),
            // 4 = flipped vertically
            4 => img.flipv(),
            // 5 = transposed (flip h + rotate 270)
            5 => img.fliph().rotate270(),
            // 6 = rotated 90 CW
            6 => img.rotate90(),
            // 7 = transverse (flip h + rotate 90)
            7 => img.fliph().rotate90(),
            // 8 = rotated 270 CW (90 CCW)
            8 => img.rotate270(),
            _ => img,
        }
    }

    /// Get cached thumbnail if exists
    pub fn get_cached(&self, source: &Path, size: u32) -> Option<PathBuf> {
        let cache_key = self.cache_key(source, size);

        if let Some(cached) = self.cache.read().get(&cache_key) {
            if cached.exists() {
                return Some(cached.clone());
            }
        }

        // Check disk cache
        let thumb_path = self.thumbnail_path(source, size);
        if thumb_path.exists() {
            self.cache.write().insert(cache_key, thumb_path.clone());
            return Some(thumb_path);
        }

        None
    }
}

impl Default for ThumbnailGenerator {
    fn default() -> Self {
        Self::new()
    }
}

/// Preview information for display
#[derive(Debug, Clone)]
pub struct PreviewInfo {
    /// Original file path
    pub source: PathBuf,
    /// Small thumbnail (64x64)
    pub thumb_small: Option<PathBuf>,
    /// Large thumbnail (512x512)
    pub thumb_large: Option<PathBuf>,
    /// MIME type
    pub mime_type: String,
    /// Is previewable
    pub previewable: bool,
    /// Error message if preview failed
    pub error: Option<String>,
}

/// Read EXIF orientation tag from an image file
///
/// Returns the EXIF orientation value (1-8), or 1 (normal) if not found.
fn read_exif_orientation(path: &Path) -> u32 {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return 1,
    };

    let mut bufreader = std::io::BufReader::new(file);
    let exif_reader = exif::Reader::new();
    let exif = match exif_reader.read_from_container(&mut bufreader) {
        Ok(e) => e,
        Err(_) => return 1,
    };

    exif.get_field(exif::Tag::Orientation, exif::In::PRIMARY)
        .and_then(|field| field.value.get_uint(0))
        .unwrap_or(1)
}

/// Check if a file is previewable (image)
pub fn is_previewable(path: &Path) -> bool {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    matches!(
        ext.as_str(),
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "ico" | "tiff" | "tif"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_thumbnail_generator_cache_key() {
        let gen = ThumbnailGenerator::new();
        let path = PathBuf::from("/test/image.jpg");

        let key1 = gen.cache_key(&path, 64);
        let key2 = gen.cache_key(&path, 512);

        assert_ne!(key1, key2);
        assert!(key1.ends_with("-64"));
        assert!(key2.ends_with("-512"));
    }

    #[test]
    fn test_is_previewable() {
        assert!(is_previewable(&PathBuf::from("test.jpg")));
        assert!(is_previewable(&PathBuf::from("test.PNG")));
        assert!(!is_previewable(&PathBuf::from("test.txt")));
        assert!(!is_previewable(&PathBuf::from("test.mp4")));
    }

    #[test]
    fn test_exif_orientation_missing_file() {
        // Non-existent file should return orientation 1 (normal)
        assert_eq!(read_exif_orientation(&PathBuf::from("/nonexistent.jpg")), 1);
    }

    #[test]
    fn test_exif_orientation_non_jpeg() {
        // Text file should return orientation 1 (no EXIF)
        let dir = tempfile::tempdir().unwrap();
        let txt_path = dir.path().join("test.txt");
        std::fs::write(&txt_path, "hello").unwrap();
        assert_eq!(read_exif_orientation(&txt_path), 1);
    }

    #[test]
    fn test_generate_progressive_multi_creates_all_sizes() {
        let gen = ThumbnailGenerator::new();

        // Create a tiny test image
        let dir = tempfile::tempdir().unwrap();
        let img_path = dir.path().join("test.png");
        let img = image::DynamicImage::new_rgb8(100, 100);
        img.save(&img_path).unwrap();

        let sizes = [32, 64, 128];
        let result = gen.generate_progressive_multi(&img_path, &sizes);
        assert!(result.is_ok());

        let paths = result.unwrap();
        assert_eq!(paths.len(), 3);
        for path in &paths {
            assert!(path.exists());
        }
    }

    #[test]
    fn test_generate_batch_parallel() {
        let gen = ThumbnailGenerator::new();
        let dir = tempfile::tempdir().unwrap();

        // Create multiple test images
        let mut sources = Vec::new();
        for i in 0..4 {
            let img_path = dir.path().join(format!("test_{}.png", i));
            let img = image::DynamicImage::new_rgb8(80, 80);
            img.save(&img_path).unwrap();
            sources.push(img_path);
        }

        let results = gen.generate_batch(&sources, 32);
        assert_eq!(results.len(), 4);
        for result in &results {
            assert!(result.is_ok());
            assert!(result.as_ref().unwrap().exists());
        }
    }
}
