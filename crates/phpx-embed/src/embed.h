/*
 * phpx - PHP CLI embedded in Rust
 * Header file for FFI bindings
 */

#ifndef PHPX_H
#define PHPX_H

#ifdef __cplusplus
extern "C" {
#endif

/*
 * Execute a PHP script file.
 * Returns the exit status code.
 */
int phpx_execute_script(const char *script_path, int argc, char **argv);

/*
 * Execute PHP code passed as a string (like php -r).
 * Returns the exit status code.
 */
int phpx_execute_code(const char *code, int argc, char **argv);

/*
 * Get PHP version string.
 */
const char *phpx_get_version(void);

/*
 * Get PHP version ID (e.g., 80300 for PHP 8.3.0).
 */
int phpx_get_version_id(void);

#ifdef __cplusplus
}
#endif

#endif /* PHPX_H */
