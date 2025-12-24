use phpx_pm::cache::Cache;
use std::time::Duration;
use tempfile::TempDir;

fn main() -> std::io::Result<()> {
    println!("Composer Cache Demo\n");

    // Create a temporary directory for the demo
    let temp_dir = TempDir::new()?;
    let cache_root = temp_dir.path().join("composer-cache");

    // Create cache instance
    let cache = Cache::new(cache_root.clone());

    println!("Cache root: {}", cache.root().display());
    println!("Cache enabled: {}", cache.is_enabled());
    println!();

    // Test 1: Write and read
    println!("1. Writing data to cache...");
    let data = b"Package metadata for symfony/console";
    cache.write("repo/packagist.org/symfony-console.json", data)?;
    println!("   Written {} bytes", data.len());

    println!("2. Reading data from cache...");
    if let Some(read_data) = cache.read("repo/packagist.org/symfony-console.json")? {
        println!("   Read {} bytes", read_data.len());
        println!("   Content: {}", String::from_utf8_lossy(&read_data));
    }
    println!();

    // Test 2: Check if file exists
    println!("3. Checking cache entries...");
    println!(
        "   Has 'repo/packagist.org/symfony-console.json': {}",
        cache.has("repo/packagist.org/symfony-console.json")
    );
    println!(
        "   Has 'nonexistent.json': {}",
        cache.has("nonexistent.json")
    );
    println!();

    // Test 3: SHA256 hash
    println!("4. Computing SHA256 hash...");
    if let Some(hash) = cache.sha256("repo/packagist.org/symfony-console.json")? {
        println!("   SHA256: {}", hash);
    }
    println!();

    // Test 4: Cache size
    println!("5. Cache statistics...");
    let size = cache.size()?;
    println!("   Total cache size: {} bytes", size);

    if let Some(age) = cache.age("repo/packagist.org/symfony-console.json")? {
        println!("   File age: {:?}", age);
    }
    println!();

    // Test 5: Write multiple files
    println!("6. Writing multiple package files...");
    cache.write("files/symfony-console-5.4.0.zip", b"[zip data]")?;
    cache.write("files/symfony-console-6.0.0.zip", b"[zip data v6]")?;
    cache.write("repo/packagist.org/symfony-http.json", b"{...}")?;
    println!("   Written 3 more files");
    println!("   New cache size: {} bytes", cache.size()?);
    println!();

    // Test 6: Garbage collection simulation
    println!("7. Simulating garbage collection...");
    println!("   Note: Files are too new, nothing will be collected");
    let freed = cache.gc(Duration::from_secs(3600))?;
    println!("   Freed {} bytes", freed);
    println!();

    // Test 7: Copy operations
    println!("8. Testing copy operations...");
    let dest_file = temp_dir.path().join("exported.json");
    if cache.copy_to("repo/packagist.org/symfony-console.json", &dest_file)? {
        println!("   Copied from cache to {}", dest_file.display());
    }

    let source_file = temp_dir.path().join("source.json");
    std::fs::write(&source_file, b"new package data")?;
    cache.copy_from("repo/new-package.json", &source_file)?;
    println!("   Copied {} to cache", source_file.display());
    println!();

    // Test 8: List cache contents
    println!("9. Current cache contents:");
    for entry in std::fs::read_dir(cache.root())? {
        let entry = entry?;
        println!("   - {}", entry.file_name().to_string_lossy());
    }
    println!();

    // Test 9: Remove a file
    println!("10. Removing a cache entry...");
    cache.remove("files/symfony-console-5.4.0.zip")?;
    println!("   Removed 'files/symfony-console-5.4.0.zip'");
    println!(
        "   Still exists: {}",
        cache.has("files/symfony-console-5.4.0.zip")
    );
    println!();

    // Test 10: Clear cache
    println!("11. Clearing entire cache...");
    cache.clear()?;
    println!("   Cache cleared");
    println!("   Cache size after clear: {} bytes", cache.size()?);
    println!();

    println!("Demo completed successfully!");

    Ok(())
}
