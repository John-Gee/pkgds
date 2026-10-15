complete -c pkgds -s H -d 'Human-size unit (B, K, Ki, M, Mi, G, Gi, T, Ti, auto, auto-si)' -r
complete -c pkgds -s s -d 'Search packages by regex on name' -r
complete -c pkgds -l sort -d 'Sort by field: name (ascending), real/apparent/files (descending), ratio (ascending)' -r
complete -c pkgds -s n -l limit -d 'Show only top N packages (0 = unlimited)' -r
complete -c pkgds -l root -d 'Set filesystem root prefix' -r -F
complete -c pkgds -l dbpath -d 'Override pacman local DB path (absolute)' -r -F
complete -c pkgds -s d -d 'Delimiter between packages in format mode' -r
complete -c pkgds -l depth -d 'Tree depth in levels (implies --tree; default 1)' -r
complete -c pkgds -l breadth -d 'Show at most K children per directory (0 = no limit; requires --tree)' -r
complete -c pkgds -l min-percent -d 'Prune entries below this percentage of their parent (requires --tree; default 0.1)' -r
complete -c pkgds -l btrfs -d 'Enable btrfs on-disk size tokens (%z, %r)'
complete -c pkgds -s v -l verbose -d 'Show warnings for missing files, permission errors'
complete -c pkgds -l no-color -d 'Disable color output'
complete -c pkgds -l apparent-size -d 'Show apparent file size instead of disk blocks (du --apparent-size)'
complete -c pkgds -l total -d 'Show total row at the bottom of table output'
complete -c pkgds -l files -d 'Show file count column'
complete -c pkgds -l tree -d 'Show a per-package file/dir tree (default depth 1)'
complete -c pkgds -l with-deps -d 'Also include the dependencies that removing the target(s) would free'
complete -c pkgds -s h -l help -d 'Print help (see more with \'--help\')'
complete -c pkgds -s V -l version -d 'Print version'
