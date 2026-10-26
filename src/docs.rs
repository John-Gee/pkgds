/// Shared extended help for `--help` and the generated man page.
///
/// Kept here (rather than inline in `config.rs`) so the CLI long help and
/// `man/pkgds.1` render the same text from one source. `-h` stays terse;
/// this block only appears in `--help` and the man page.
pub const AFTER_LONG_HELP: &str = "\
FORMAT
  Pass a string containing `%` as the first argument to print one line per
  package with exactly the fields you ask for. Tokens: %n name, %v version,
  %m real disk usage, %a apparent size, %f file count, %p metadata size,
  %z btrfs on-disk usage (requires --btrfs), %r btrfs ratio, %% literal %.
  `\\t` and `\\n` inside the format string become tab and newline. `-d`
  is used literally between packages; use shell $'\\t' for a tab there.

MODES
  Table mode (default) shows the top 20 packages; format mode shows all
  matches unless -n is given. -H defaults to auto in table mode and raw
  bytes in format mode. The % share column is omitted when only one
  package matches. A format string takes precedence over --tree. --total
  and --files describe the table and are ignored in the tree view. With
  --btrfs the tree is measured in on-disk bytes, so a package with no
  on-disk data is left out and reported on stderr rather than shown.

SELECTION
  Named --with-deps expansion is described under WITH-DEPS MODES below.
  --sort ratio requires --btrfs. Flags may follow package names
  (`pkgds bash --btrfs`); a format string starting with `-` needs `--` first.

WITH-DEPS MODES
  With targets (names or -s/--search) --with-deps expands the selection to
  what removing them would free: the targets plus every dependency that was
  installed as a dependency and that nothing else needs. TOTAL is the joint
  freed bytes.
  Bare --with-deps (no targets, no -s/--search, no format string) ranks
  installed packages (top 20 shown by default, -n 0 for all): one row per
  shown package, REAL untouched, SAVES is the space
  that removing only that package would free (the package plus the deps that
  would become unneeded), or zero when something still installed requires
  it (removal would be refused). SAVES uses the shown size (REAL, or
  APPARENT with
  --apparent-size) summed over that set, so a file claimed by two packages
  counts in each claim and SAVES values overlap; TOTAL and the % column stay
  off in this mode. --sort is ignored (rank is by SAVES, then -n applies).
  Table only: bare --with-deps with a format string or --tree is an error.
  --btrfs DISK and RATIO stay per package.

EXIT CODES
  0 success. 1 validated usage error (bad flag combination, missing paths)
  or nothing matched at all. 2 unknown flag or bad value (argument parse
  error), or partial result: some file could not be read, some package was
  malformed, btrfs data was unavailable, or some requested package names
  were unknown. A full scan exits 2 whenever any package owns a file this
  user cannot read, so treat 2 as \"numbers are a lower bound\",
  not as failure.\
";
