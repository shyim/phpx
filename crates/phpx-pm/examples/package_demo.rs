use phpx_pm::package::{
    Abandoned, Autoload, Author, Dist, Link, LinkType, Package, Source, Stability, Support,
};

fn main() {
    println!("=== Composer Package Model Demo ===\n");

    // Example 1: Creating a basic package
    println!("1. Creating a basic package:");
    let mut package = Package::new("vendor/my-library", "1.2.3");
    package.description = Some("A useful PHP library".to_string());
    package.homepage = Some("https://github.com/vendor/my-library".to_string());
    package.license = vec!["MIT".to_string()];
    package.keywords = vec!["php".to_string(), "library".to_string()];

    println!("   Name: {}", package.pretty_string());
    println!("   Type: {}", package.package_type());
    println!("   Stability: {}", package.stability());
    println!();

    // Example 2: Adding dependencies
    println!("2. Adding dependencies:");
    package.require.insert("php".to_string(), ">=8.1".to_string());
    package
        .require
        .insert("symfony/console".to_string(), "^6.0".to_string());
    package.require_dev.insert(
        "phpunit/phpunit".to_string(),
        "^10.0".to_string(),
    );

    println!("   Runtime dependencies:");
    for (name, version) in &package.require {
        println!("     - {} {}", name, version);
    }
    println!("   Development dependencies:");
    for (name, version) in &package.require_dev {
        println!("     - {} {}", name, version);
    }
    println!();

    // Example 3: Converting to Link objects
    println!("3. Converting dependencies to Link objects:");
    let links = package.get_links();
    for link in links.iter().take(3) {
        println!("   {}", link);
    }
    println!();

    // Example 4: Adding source information
    println!("4. Adding source information (Git):");
    let source = Source::git(
        "https://github.com/vendor/my-library.git",
        "abc123def456",
    );
    package.source = Some(source);
    println!(
        "   Source: {} @ {}",
        package.source.as_ref().unwrap().url,
        package.source.as_ref().unwrap().reference
    );
    println!();

    // Example 5: Adding distribution information
    println!("5. Adding distribution information (ZIP archive):");
    let dist = Dist::zip("https://api.github.com/repos/vendor/my-library/zipball/abc123def456")
        .with_reference("abc123def456")
        .with_shasum("1234567890abcdef1234567890abcdef12345678");
    package.dist = Some(dist);
    println!(
        "   Dist: {} ({})",
        package.dist.as_ref().unwrap().url,
        package.dist.as_ref().unwrap().dist_type
    );
    println!(
        "   Checksum: {}",
        package.dist.as_ref().unwrap().shasum.as_ref().unwrap()
    );
    println!();

    // Example 6: Adding autoload configuration
    println!("6. Configuring autoload:");
    let autoload = Autoload::new()
        .add_psr4("Vendor\\MyLibrary\\", "src/")
        .add_file("src/functions.php");
    package.autoload = Some(autoload);

    let autoload_dev = Autoload::new().add_psr4("Vendor\\MyLibrary\\Tests\\", "tests/");
    package.autoload_dev = Some(autoload_dev);

    println!("   PSR-4 namespaces:");
    if let Some(autoload) = &package.autoload {
        for (namespace, paths) in &autoload.psr4 {
            println!("     {} => {:?}", namespace, paths.as_vec());
        }
    }
    println!();

    // Example 7: Adding author information
    println!("7. Adding author information:");
    let author = Author {
        name: Some("John Doe".to_string()),
        email: Some("john@example.com".to_string()),
        homepage: Some("https://johndoe.dev".to_string()),
        role: Some("Developer".to_string()),
    };
    package.authors = vec![author];
    for author in &package.authors {
        println!(
            "   {} <{}> ({})",
            author.name.as_ref().unwrap(),
            author.email.as_ref().unwrap(),
            author.role.as_ref().unwrap()
        );
    }
    println!();

    // Example 8: Adding support information
    println!("8. Adding support information:");
    let support = Support {
        issues: Some("https://github.com/vendor/my-library/issues".to_string()),
        source: Some("https://github.com/vendor/my-library".to_string()),
        docs: Some("https://my-library.readthedocs.io".to_string()),
        ..Default::default()
    };
    package.support = Some(support);
    println!("   Issues: {}", package.support.as_ref().unwrap().issues.as_ref().unwrap());
    println!("   Documentation: {}", package.support.as_ref().unwrap().docs.as_ref().unwrap());
    println!();

    // Example 9: Adding scripts
    println!("9. Adding Composer scripts:");
    package.scripts.insert(
        "test".to_string(),
        phpx_pm::package::ScriptHandler::Single("phpunit".to_string()),
    );
    package.scripts.insert(
        "cs".to_string(),
        phpx_pm::package::ScriptHandler::Multiple(vec![
            "php-cs-fixer fix --dry-run".to_string(),
            "phpstan analyze".to_string(),
        ]),
    );
    for (event, handler) in &package.scripts {
        println!("   {}: {:?}", event, handler);
    }
    println!();

    // Example 10: Creating a dev version
    println!("10. Creating a development version:");
    let dev_package = Package::new("vendor/dev-package", "dev-main");
    println!("   Name: {}", dev_package.pretty_string());
    println!("   Is dev: {}", dev_package.is_dev());
    println!("   Stability: {}", dev_package.stability());
    println!();

    // Example 11: Abandoned package
    println!("11. Marking a package as abandoned:");
    let mut old_package = Package::new("vendor/old-package", "2.0.0");
    old_package.abandoned = Some(Abandoned::Replacement("vendor/new-package".to_string()));
    println!("   Package: {}", old_package.pretty_string());
    println!("   Is abandoned: {}", old_package.is_abandoned());
    if let Some(abandoned) = &old_package.abandoned {
        if let Some(replacement) = abandoned.replacement() {
            println!("   Replacement: {}", replacement);
        }
    }
    println!();

    // Example 12: Serialization to JSON
    println!("12. Serializing package to JSON:");
    let json = serde_json::to_string_pretty(&package).unwrap();
    println!("{}", json);
    println!();

    // Example 13: Creating Link objects manually
    println!("13. Creating Link objects:");
    let link = Link::new(
        "my/package",
        "vendor/library",
        "^2.0",
        LinkType::Require,
    );
    println!("   {}", link);

    let dev_link = Link::new(
        "my/package",
        "phpunit/phpunit",
        "^10.0",
        LinkType::DevRequire,
    );
    println!("   {}", dev_link);

    let conflict_link = Link::new(
        "my/package",
        "vendor/bad-library",
        "*",
        LinkType::Conflict,
    );
    println!("   {}", conflict_link);
    println!();

    // Example 14: Stability comparison
    println!("14. Stability comparison:");
    let stabilities = vec![
        Stability::Dev,
        Stability::Alpha,
        Stability::Beta,
        Stability::RC,
        Stability::Stable,
    ];
    for stability in &stabilities {
        println!(
            "   {}: priority {}",
            stability,
            stability.priority()
        );
    }
    println!();

    println!("=== Demo Complete ===");
}
