_pkgds() {
    local i cur prev opts cmd
    COMPREPLY=()
    if [[ "${BASH_VERSINFO[0]}" -ge 4 ]]; then
        cur="$2"
    else
        cur="${COMP_WORDS[COMP_CWORD]}"
    fi
    prev="$3"
    cmd=""
    opts=""

    for i in "${COMP_WORDS[@]:0:COMP_CWORD}"
    do
        case "${cmd},${i}" in
            ",$1")
                cmd="pkgds"
                ;;
            *)
                ;;
        esac
    done

    case "${cmd}" in
        pkgds)
            opts="-H -s -n -d -v -h -V --search --sort --limit --btrfs --root --dbpath --verbose --no-color --apparent-size --total --files --tree --depth --breadth --min-percent --with-deps --help --version"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 1 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                -H)
                    COMPREPLY=($(compgen -W "B K Ki M Mi G Gi T Ti auto auto-si" -- "${cur}"))
                    return 0
                    ;;
                --search)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -s)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --sort)
                    COMPREPLY=($(compgen -W "name real size apparent files ratio" -- "${cur}"))
                    return 0
                    ;;
                --limit)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -n)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --root)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --dbpath)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -d)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --depth)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --breadth)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --min-percent)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
    esac
}

if [[ "${BASH_VERSINFO[0]}" -eq 4 && "${BASH_VERSINFO[1]}" -ge 4 || "${BASH_VERSINFO[0]}" -gt 4 ]]; then
    complete -F _pkgds -o nosort -o bashdefault -o default pkgds
else
    complete -F _pkgds -o bashdefault -o default pkgds
fi
