# pkgds - Real Disk Usage per Package for Arch Linux

[![AUR](https://img.shields.io/aur/version/pkgds)](https://aur.archlinux.org/packages/pkgds)

`pkgds` reports the **real on-disk size** of each installed Arch Linux package:
what is actually allocated, including block rounding, compression, hardlinks and
post-install changes. `expac -Q '%m'` reports only pacman's *declared* size from
package metadata, which is often wrong.

```
$ pkgds -H Gi rocm-llvm miopen-hip rocblas
PACKAGE        REAL      %
rocm-llvm   6.9 GiB  78.1%
miopen-hip  1.5 GiB  16.6%
rocblas     0.5 GiB   5.3%

$ expac -H G '%n %m' rocm-llvm miopen-hip rocblas   # declared size
rocm-llvm 6.86 GiB
miopen-hip 2.28 GiB
rocblas 1.29 GiB
```

`rocblas` uses barely a third of the space its metadata claims, because the ROCm
packages share hardlinked libraries. A full scan of ~2500 packages (~600k files)
takes under a second warm on NVMe.

## Installation

`pkgds` is available from the [AUR](https://aur.archlinux.org/packages/pkgds).
Without an AUR helper:

```bash
git clone https://aur.archlinux.org/pkgds.git
cd pkgds
makepkg -si
```

The package installs the `pkgds(1)` man page and bash, zsh and fish
completions.

## Usage

```
pkgds [OPTIONS] [POSITIONAL]...
```

If the first positional argument contains a `%`, it's treated as the format
string; otherwise all positionals are package targets and the rich table is
printed.

### Examples

```bash
# Top 20 packages by real disk usage
pkgds

# All packages, apparent size, human-readable
pkgds -n 0 -H auto --apparent-size

# Metadata vs real size side by side
pkgds -H auto "%n\t%p\t%m"

# Expand the biggest packages into file/dir trees
pkgds --tree --depth 3 --min-percent 1

# btrfs on-disk sizes (requires root)
sudo pkgds --btrfs -H auto

# Pipe-friendly: no color, raw format
pkgds -H Mi "%n\t%m" | sort -k2 -rn | head -5

# Scan a chroot
pkgds --root /mnt/arch --dbpath /mnt/arch/var/lib/pacman -H auto

# A package plus the dependencies removing it would free
pkgds --with-deps -H auto git

# Rank all packages by space that removing only that package would free
pkgds --with-deps -H auto
```

### Options

| Flag | Default | Description |
|------|---------|-------------|
| `[POSITIONAL]...` | (table mode) | Format string or package names |
| `-H <unit>` | `raw` (format) / `auto` (table) | Human-size: `B`, `K`/`Ki`, `M`/`Mi`, `G`/`Gi`, `T`/`Ti`, `auto`, `auto-si` |
| `-s, --search <regex>` | none | Search packages by regex on name |
| `--sort <field>` | `real` | Sort by: `name` (ascending A-Z), `real` (alias `size`)/`apparent`/`files` (descending), `ratio` (ascending, best compression first; requires `--btrfs`) |
| `-n, --limit <N>` | 20 (table) / unlimited (format) | Show only top N packages (0 = unlimited) |
| `--btrfs` | off | Enable btrfs on-disk size tokens (`%z`, `%r`) |
| `--root <path>` | `/` | Set filesystem root prefix |
| `--dbpath <path>` | none | Override pacman local DB path (absolute) |
| `-d <delim>` | newline | Delimiter between packages in format mode (used literally; `-d $'\t'` for a tab) |
| `-v, --verbose` | off | Show warnings for missing files, permission errors |
| `--no-color` | auto | Disable color output (also honors `NO_COLOR`) |
| `--apparent-size` | none | Show apparent file size instead of disk blocks |
| `--total` | none | Show a TOTAL row; when `-n` truncates the list, also a SHOWN subtotal |
| `--files` | none | Show file count column |
| `--with-deps` | off | With targets include freed dependencies; bare ranks all packages by freed space (table only) |
| `--tree` | off | Expand each shown package into a file/dir tree (default depth 1) |
| `--depth <N>` | 1 (with `--tree`) | Tree depth in levels; implies `--tree` |
| `--breadth <K>` | 5 | Show at most K children per tree directory (0 = no limit; requires `--tree`) |
| `--min-percent <P>` | 0.1 | Hide tree entries below P% of their parent (requires `--tree`) |
| `-V, --version` | none | Print version |
| `-h, --help` | none | Print help |

`%` is each package's share of the total size of all matching packages; it's
omitted when only one package matches. Flags may follow package names
(`pkgds bash --btrfs`); a format string starting with `-` needs `--` first.

`--tree` replaces the table; `--total` and `--files` describe the table and
are ignored in the tree view, and a format string takes precedence over
`--tree`. With `--btrfs` the tree is measured in on-disk bytes, so a package
with no on-disk data is left out and reported on stderr rather than shown in
a second unit.

`--with-deps` has two modes. With targets (names or `-s`/`--search` matches)
it expands the selection to what removing them would free: the packages
themselves plus every dependency that was installed as a dependency and that
nothing else needs. Shared dependencies such as glibc are therefore not
counted, and the TOTAL row is the sum of those packages' usage.

Bare `--with-deps` (no targets, no `-s`/`--search`, no format string) ranks
every installed package: one row per package, REAL untouched, SAVES is the
space that removing only that package would free (the package plus the deps
that would become unneeded). SAVES uses the shown size (REAL, or APPARENT with
`--apparent-size`) summed over that set, so a file claimed by two packages
counts in each claim and SAVES values overlap; TOTAL and the `%` column stay
off in this mode. `--sort` is ignored (rank is by SAVES, then `-n` applies).
Table only: bare `--with-deps` with a format string or `--tree` is an error.
`--btrfs` DISK and RATIO stay per package.

### Format Tokens

Pass a string containing `%` as the first argument to print one line per package
with exactly the fields you ask for: useful for scripting and pipes.

| Token | Field | Source |
|-------|-------|--------|
| `%n` | Package name | `desc` file `%NAME%` |
| `%v` | Package version | `desc` file `%VERSION%` |
| `%m` | Real disk usage | `stat() st_blocks * 512`; hardlinked inodes counted once per package, symlinks skipped |
| `%a` | Apparent size | `stat() st_size`; same rules as `%m` |
| `%f` | File count | unique regular files (hardlinked inodes counted once) |
| `%p` | Metadata size | `desc` file `%SIZE%` |
| `%z` | btrfs on-disk usage | all file extents via btrfs ioctls, compressed or not (requires `--btrfs` + root; only files on the subvolume `--root` lives in) |
| `%r` | btrfs ratio | `(%z / %a) * 100`; 100% = uncompressed |
| `%%` | Literal `%` | none |

Sizes come from the manifest rather than a directory walk, so three things
differ from `du`: symlinks are skipped, a hardlinked inode is counted once
*per package* (so two packages sharing a library each claim it, and a
`--total` grand total is the sum of those claims), and files a package no
longer owns are excluded.

## Exit Codes

0 success. 1 usage error or nothing matched. 2 partial result (some file could
not be read, btrfs data was unavailable, or some names were unknown); treat 2
as "numbers are a lower bound", not as failure. See `pkgds --help` and
`pkgds(1)` for the full list.
