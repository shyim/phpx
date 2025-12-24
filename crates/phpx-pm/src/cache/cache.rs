use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use walkdir::WalkDir;

/// Filesystem cache for composer data
///
/// Supports caching of:
/// - files/ - Downloaded package archives
/// - repo/ - Repository metadata (packages.json, etc.)
/// - vcs/ - VCS clones
pub struct Cache {
    /// Root directory of the cache
    root: PathBuf,
    /// Whether the cache is enabled
    enabled: bool,
    /// Whether the cache is read-only
    read_only: bool,
    /// Characters allowed in cache keys (used for sanitization)
    allowlist: String,
}

impl Cache {
    /// Create a new cache instance
    ///
    /// # Arguments
    /// * `root` - Root directory for cache storage
    ///
    /// # Example
    /// ```no_run
    /// use std::path::PathBuf;
    /// use phpx_pm::cache::Cache;
    ///
    /// let cache = Cache::new(PathBuf::from("/tmp/composer-cache"));
    /// ```
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            enabled: true,
            read_only: false,
            allowlist: "a-z0-9._".to_string(),
        }
    }

    /// Create a new cache with custom allowlist
    ///
    /// # Arguments
    /// * `root` - Root directory for cache storage
    /// * `allowlist` - Characters allowed in cache keys (regex character class)
    pub fn with_allowlist(root: PathBuf, allowlist: String) -> Self {
        Self {
            root,
            enabled: true,
            read_only: false,
            allowlist,
        }
    }

    /// Set the read-only mode
    pub fn set_read_only(&mut self, read_only: bool) {
        self.read_only = read_only;
    }

    /// Check if cache is read-only
    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// Set whether cache is enabled
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Check if cache is enabled
    pub fn is_enabled(&self) -> bool {
        if !self.enabled {
            return false;
        }

        // Check if cache path is usable (not /dev/null, nul, etc.)
        if !Self::is_usable(&self.root) {
            return false;
        }

        // If read-only, just check if directory exists
        if self.read_only {
            return self.root.is_dir();
        }

        // For writable cache, ensure directory exists and is writable
        if !self.root.exists() {
            if fs::create_dir_all(&self.root).is_err() {
                return false;
            }
        }

        // Check if writable by attempting to create a test file
        let test_file = self.root.join(".cache_test");
        if File::create(&test_file).is_ok() {
            let _ = fs::remove_file(&test_file);
            true
        } else {
            false
        }
    }

    /// Check if a cache path is usable (not /dev/null, nul, etc.)
    pub fn is_usable(path: &Path) -> bool {
        let path_str = path.to_string_lossy();

        // Check for common null device patterns
        if path_str.contains("/dev/null")
            || path_str.contains("\\dev\\null")
            || path_str.contains("nul")
            || path_str.contains("NUL")
            || path_str.contains("$null") {
            return false;
        }

        true
    }

    /// Get the root directory of the cache
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Sanitize a cache key to ensure it's safe for filesystem use
    ///
    /// Replaces characters not in the allowlist with dashes
    fn sanitize_key(&self, key: &str) -> String {
        let pattern = format!("[^{}]", self.allowlist);
        let re = regex::Regex::new(&pattern).unwrap();
        re.replace_all(key, "-").to_string()
    }

    /// Get the full path for a cache key
    fn get_path(&self, key: &str) -> PathBuf {
        let sanitized = self.sanitize_key(key);
        self.root.join(sanitized)
    }

    /// Check if a file exists in the cache
    ///
    /// # Arguments
    /// * `key` - Cache key to check
    pub fn has(&self, key: &str) -> bool {
        if !self.is_enabled() {
            return false;
        }

        let path = self.get_path(key);
        path.exists() && path.is_file()
    }

    /// Read data from cache
    ///
    /// # Arguments
    /// * `key` - Cache key to read
    ///
    /// # Returns
    /// * `Ok(Some(data))` - Data was found and read successfully
    /// * `Ok(None)` - Cache is disabled or key doesn't exist
    /// * `Err(e)` - IO error occurred
    pub fn read(&self, key: &str) -> io::Result<Option<Vec<u8>>> {
        if !self.is_enabled() {
            return Ok(None);
        }

        let path = self.get_path(key);
        if !path.exists() {
            return Ok(None);
        }

        let data = fs::read(&path)?;
        Ok(Some(data))
    }

    /// Write data to cache
    ///
    /// Uses atomic write (write to temp file, then rename) to ensure consistency
    ///
    /// # Arguments
    /// * `key` - Cache key to write
    /// * `data` - Data to write
    pub fn write(&self, key: &str, data: &[u8]) -> io::Result<()> {
        if !self.is_enabled() || self.read_only {
            return Ok(());
        }

        let path = self.get_path(key);

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Write to temporary file first (atomic write)
        let temp_path = path.with_extension("tmp");
        {
            let mut file = File::create(&temp_path)?;
            file.write_all(data)?;
            file.sync_all()?;
        }

        // Rename to final location (atomic on most filesystems)
        fs::rename(&temp_path, &path)?;

        Ok(())
    }

    /// Copy a file from cache to destination
    ///
    /// # Arguments
    /// * `key` - Cache key to copy from
    /// * `dest` - Destination path
    ///
    /// # Returns
    /// * `Ok(true)` - File was copied successfully
    /// * `Ok(false)` - Cache is disabled or key doesn't exist
    /// * `Err(e)` - IO error occurred
    pub fn copy_to(&self, key: &str, dest: &Path) -> io::Result<bool> {
        if !self.is_enabled() {
            return Ok(false);
        }

        let path = self.get_path(key);
        if !path.exists() {
            return Ok(false);
        }

        // Update access time to help with LRU eviction
        let _ = self.touch(&path);

        // Ensure destination directory exists
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }

        fs::copy(&path, dest)?;
        Ok(true)
    }

    /// Copy a file to cache
    ///
    /// # Arguments
    /// * `key` - Cache key to store under
    /// * `source` - Source file path
    pub fn copy_from(&self, key: &str, source: &Path) -> io::Result<()> {
        if !self.is_enabled() || self.read_only {
            return Ok(());
        }

        if !source.exists() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("Source file does not exist: {}", source.display()),
            ));
        }

        let path = self.get_path(key);

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        fs::copy(source, &path)?;
        Ok(())
    }

    /// Delete a file from cache
    ///
    /// # Arguments
    /// * `key` - Cache key to delete
    pub fn remove(&self, key: &str) -> io::Result<()> {
        if !self.is_enabled() || self.read_only {
            return Ok(());
        }

        let path = self.get_path(key);
        if path.exists() {
            fs::remove_file(&path)?;
        }

        Ok(())
    }

    /// Clear the entire cache
    ///
    /// Removes all files and directories under the cache root
    pub fn clear(&self) -> io::Result<()> {
        if !self.is_enabled() || self.read_only {
            return Ok(());
        }

        // Remove all contents but keep the root directory
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_dir() {
                fs::remove_dir_all(&path)?;
            } else {
                fs::remove_file(&path)?;
            }
        }

        Ok(())
    }

    /// Garbage collect old cache entries
    ///
    /// Removes files older than the specified TTL
    ///
    /// # Arguments
    /// * `ttl` - Time-to-live duration
    ///
    /// # Returns
    /// Number of bytes freed
    pub fn gc(&self, ttl: Duration) -> io::Result<u64> {
        if !self.is_enabled() || self.read_only {
            return Ok(0);
        }

        let now = SystemTime::now();
        let mut freed = 0u64;

        for entry in WalkDir::new(&self.root)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();

            // Skip directories
            if !path.is_file() {
                continue;
            }

            // Get file metadata
            if let Ok(metadata) = fs::metadata(path) {
                // Check if file is older than TTL
                if let Ok(modified) = metadata.modified() {
                    if let Ok(age) = now.duration_since(modified) {
                        if age > ttl {
                            let size = metadata.len();
                            if fs::remove_file(path).is_ok() {
                                freed += size;
                            }
                        }
                    }
                }
            }
        }

        Ok(freed)
    }

    /// Garbage collect VCS cache directories
    ///
    /// Removes VCS cache directories older than the specified TTL
    ///
    /// # Arguments
    /// * `ttl` - Time-to-live duration
    ///
    /// # Returns
    /// Number of bytes freed
    pub fn gc_vcs(&self, ttl: Duration) -> io::Result<u64> {
        if !self.is_enabled() || self.read_only {
            return Ok(0);
        }

        let now = SystemTime::now();
        let mut freed = 0u64;

        // Only check immediate subdirectories (depth 0)
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let path = entry.path();

            // Only process directories
            if !path.is_dir() {
                continue;
            }

            // Get directory metadata
            if let Ok(metadata) = fs::metadata(&path) {
                // Check if directory is older than TTL
                if let Ok(modified) = metadata.modified() {
                    if let Ok(age) = now.duration_since(modified) {
                        if age > ttl {
                            // Calculate directory size before removal
                            let size = self.dir_size(&path)?;
                            if fs::remove_dir_all(&path).is_ok() {
                                freed += size;
                            }
                        }
                    }
                }
            }
        }

        Ok(freed)
    }

    /// Get SHA256 hash of a cached file
    ///
    /// # Arguments
    /// * `key` - Cache key to hash
    ///
    /// # Returns
    /// * `Ok(Some(hash))` - File exists and hash was computed
    /// * `Ok(None)` - Cache is disabled or file doesn't exist
    /// * `Err(e)` - IO error occurred
    pub fn sha256(&self, key: &str) -> io::Result<Option<String>> {
        if !self.is_enabled() {
            return Ok(None);
        }

        let path = self.get_path(key);
        if !path.exists() {
            return Ok(None);
        }

        let mut file = File::open(&path)?;
        let mut hasher = Sha256::new();
        let mut buffer = [0u8; 8192];

        loop {
            let n = file.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
        }

        let result = hasher.finalize();
        Ok(Some(format!("{:x}", result)))
    }

    /// Get the total size of the cache
    ///
    /// # Returns
    /// Total size in bytes
    pub fn size(&self) -> io::Result<u64> {
        if !self.is_enabled() {
            return Ok(0);
        }

        self.dir_size(&self.root)
    }

    /// Get the age of a cached file
    ///
    /// # Arguments
    /// * `key` - Cache key to check
    ///
    /// # Returns
    /// Age in seconds, or None if file doesn't exist
    pub fn age(&self, key: &str) -> io::Result<Option<Duration>> {
        if !self.is_enabled() {
            return Ok(None);
        }

        let path = self.get_path(key);
        if !path.exists() {
            return Ok(None);
        }

        let metadata = fs::metadata(&path)?;
        let modified = metadata.modified()?;
        let now = SystemTime::now();

        match now.duration_since(modified) {
            Ok(duration) => Ok(Some(duration)),
            Err(_) => Ok(None),
        }
    }

    /// Touch a file to update its access time
    fn touch(&self, path: &Path) -> io::Result<()> {
        // On Unix systems, we can use filetime crate, but for simplicity
        // we'll just try to update metadata using a platform-independent approach

        // Try to open and close the file to update access time
        let _ = File::open(path)?;

        Ok(())
    }

    /// Calculate the total size of a directory
    fn dir_size(&self, path: &Path) -> io::Result<u64> {
        let mut total = 0u64;

        for entry in WalkDir::new(path)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if let Ok(metadata) = entry.metadata() {
                if metadata.is_file() {
                    total += metadata.len();
                }
            }
        }

        Ok(total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration as StdDuration;
    use tempfile::TempDir;

    #[test]
    fn test_cache_new() {
        let temp = TempDir::new().unwrap();
        let cache = Cache::new(temp.path().to_path_buf());

        assert_eq!(cache.root(), temp.path());
        assert!(cache.is_enabled());
        assert!(!cache.is_read_only());
    }

    #[test]
    fn test_cache_read_write() {
        let temp = TempDir::new().unwrap();
        let cache = Cache::new(temp.path().to_path_buf());

        let data = b"Hello, World!";
        cache.write("test.txt", data).unwrap();

        let read_data = cache.read("test.txt").unwrap();
        assert_eq!(read_data, Some(data.to_vec()));
    }

    #[test]
    fn test_cache_has() {
        let temp = TempDir::new().unwrap();
        let cache = Cache::new(temp.path().to_path_buf());

        assert!(!cache.has("test.txt"));

        cache.write("test.txt", b"data").unwrap();
        assert!(cache.has("test.txt"));
    }

    #[test]
    fn test_cache_remove() {
        let temp = TempDir::new().unwrap();
        let cache = Cache::new(temp.path().to_path_buf());

        cache.write("test.txt", b"data").unwrap();
        assert!(cache.has("test.txt"));

        cache.remove("test.txt").unwrap();
        assert!(!cache.has("test.txt"));
    }

    #[test]
    fn test_cache_copy_to() {
        let temp = TempDir::new().unwrap();
        let cache = Cache::new(temp.path().join("cache"));

        cache.write("test.txt", b"Hello").unwrap();

        let dest = temp.path().join("output.txt");
        let result = cache.copy_to("test.txt", &dest).unwrap();

        assert!(result);
        assert!(dest.exists());
        assert_eq!(fs::read(&dest).unwrap(), b"Hello");
    }

    #[test]
    fn test_cache_copy_from() {
        let temp = TempDir::new().unwrap();
        let cache = Cache::new(temp.path().join("cache"));

        let source = temp.path().join("source.txt");
        fs::write(&source, b"Hello").unwrap();

        cache.copy_from("test.txt", &source).unwrap();

        assert!(cache.has("test.txt"));
        assert_eq!(cache.read("test.txt").unwrap().unwrap(), b"Hello");
    }

    #[test]
    fn test_cache_sha256() {
        let temp = TempDir::new().unwrap();
        let cache = Cache::new(temp.path().to_path_buf());

        cache.write("test.txt", b"Hello, World!").unwrap();

        let hash = cache.sha256("test.txt").unwrap();
        assert!(hash.is_some());

        // SHA256 of "Hello, World!"
        let expected = "dffd6021bb2bd5b0af676290809ec3a53191dd81c7f70a4b28688a362182986f";
        assert_eq!(hash.unwrap(), expected);
    }

    #[test]
    fn test_cache_clear() {
        let temp = TempDir::new().unwrap();
        let cache = Cache::new(temp.path().to_path_buf());

        cache.write("test1.txt", b"data1").unwrap();
        cache.write("test2.txt", b"data2").unwrap();

        assert!(cache.has("test1.txt"));
        assert!(cache.has("test2.txt"));

        cache.clear().unwrap();

        assert!(!cache.has("test1.txt"));
        assert!(!cache.has("test2.txt"));
    }

    #[test]
    fn test_cache_gc() {
        let temp = TempDir::new().unwrap();
        let cache = Cache::new(temp.path().to_path_buf());

        cache.write("old.txt", b"old data").unwrap();

        // Wait a bit
        thread::sleep(StdDuration::from_millis(100));

        cache.write("new.txt", b"new data").unwrap();

        // GC with very short TTL should remove old file
        let freed = cache.gc(Duration::from_millis(50)).unwrap();
        assert!(freed > 0);

        assert!(!cache.has("old.txt"));
        assert!(cache.has("new.txt"));
    }

    #[test]
    fn test_cache_size() {
        let temp = TempDir::new().unwrap();
        let cache = Cache::new(temp.path().to_path_buf());

        cache.write("test1.txt", b"Hello").unwrap();
        cache.write("test2.txt", b"World!").unwrap();

        let size = cache.size().unwrap();
        assert_eq!(size, 11); // "Hello" (5) + "World!" (6)
    }

    #[test]
    fn test_cache_sanitize_key() {
        let temp = TempDir::new().unwrap();
        let cache = Cache::new(temp.path().to_path_buf());

        // Keys with special characters should be sanitized
        cache.write("test/with/slashes", b"data").unwrap();
        assert!(cache.has("test/with/slashes"));

        // The actual file should have dashes instead of slashes
        let sanitized_path = cache.get_path("test/with/slashes");
        assert!(sanitized_path.to_string_lossy().contains("test-with-slashes"));
    }

    #[test]
    fn test_cache_read_only() {
        let temp = TempDir::new().unwrap();
        let mut cache = Cache::new(temp.path().to_path_buf());

        // Write some data first
        cache.write("test.txt", b"data").unwrap();

        // Set to read-only
        cache.set_read_only(true);
        assert!(cache.is_read_only());

        // Writing should be no-op
        cache.write("test2.txt", b"data2").unwrap();
        assert!(!cache.has("test2.txt"));

        // Reading should still work
        let data = cache.read("test.txt").unwrap();
        assert_eq!(data, Some(b"data".to_vec()));
    }

    #[test]
    fn test_is_usable() {
        assert!(!Cache::is_usable(Path::new("/dev/null")));
        assert!(!Cache::is_usable(Path::new("nul")));
        assert!(!Cache::is_usable(Path::new("NUL")));
        assert!(!Cache::is_usable(Path::new("$null")));

        assert!(Cache::is_usable(Path::new("/tmp/cache")));
        assert!(Cache::is_usable(Path::new("./cache")));
    }
}
