_argc_require_bash() {
    local major minor patch required current
    IFS=. read -r major minor patch <<<"$1"
    required=$((major * 1000000 + ${minor:-0} * 1000 + ${patch:-0}))
    current=$((BASH_VERSINFO[0] * 1000000 + BASH_VERSINFO[1] * 1000 + BASH_VERSINFO[2]))
    if [ "$current" -lt "$required" ]; then
        echo "error: bash $1+ is required, found $BASH_VERSION" >&2
        exit 1
    fi
}
