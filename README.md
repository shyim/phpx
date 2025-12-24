# PHPx

## This is TOTALLY WIP and experimental

PHPx (temporary name) is an idea to build a all-in-one single binary PHP distribution, which contains all PHP extensions, a Webserver, Package Manager, Formatter/Linter for best development experience working with PHP.

Ideas:

- Production Ready Webserver
- Package Manager that is equal to Composer
- Formatter/Linter that uses Mago
- A builtin test runner?

## Current Status

### General

- [_] A `phpx.toml` file to configure PHP settings like `memory_limit` or other things

### Web Server

- [X] Regular Webserver
- [X] Worker Mode similar to FrankenPHP
- [_] Production Ready

### Package Manager

- [X] Install Packages
- [X] Update Packages
- [X] Remove Packages
- [X] Audit Packages
- [_] Composer Plugins

### Formatter/Linter

- [_] Formatter/Linter that uses Mago

## The CLI

```
phpx 0.1.0 - PHP 8.5.1 embedded in Rust

Usage: phpx [options] [-f] <file> [--] [args...]
       phpx [options] -r <code> [--] [args...]
       phpx server [options] [router.php]

Options:
  -d key[=value]  Define INI entry
  -i              PHP information (phpinfo)
  -l              Syntax check only (lint)
  -m              Show compiled in modules
  -r <code>       Run PHP <code> without script tags
  -v              Version information
  -h, --help      Show this help message

Subcommands:
  init            Create a new composer.json in current directory
  install         Install project dependencies from composer.lock
  update          Update dependencies to their latest versions
  add             Add a package to the project
  remove          Remove a package from the project
  run             Run a script defined in composer.json
  server          Start a PHP development server
  pm              Other package manager commands (show, validate, etc.)

Run 'phpx --help' for more options.
```

### PM Commands

```
❯ phpx pm
Package manager commands (show, validate, dump-autoload)

Usage: phpx pm <COMMAND>

Commands:
  audit          Check for security vulnerabilities in installed packages
  bump           Bump version constraints in composer.json to locked versions
  exec           Execute a vendored binary/script
  search         Search for packages on Packagist
  show           Show information about packages
  validate       Validate composer.json and composer.lock
  dump-autoload  Regenerate the autoloader
  why            Show why a package is installed
  outdated       Show outdated packages
  clear-cache    Clear the Composer cache
  help           Print this message or the help of the given subcommand(s)

Options:
  -h, --help  Print help
```
