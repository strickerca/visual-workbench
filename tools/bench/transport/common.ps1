function Get-TransportHash([string]$Path) {
    $stream = [IO.File]::OpenRead($Path)
    $sha = [Security.Cryptography.SHA256]::Create()
    try { return ([BitConverter]::ToString($sha.ComputeHash($stream))).Replace('-','').ToLowerInvariant() }
    finally { $sha.Dispose(); $stream.Dispose() }
}

function Get-TransportCleanupCommand([string]$Directory, [string]$PidFile) {
    if ($Directory -notmatch '^/data/local/tmp/vw-transport-[0-9a-f]{32}$' -or $PidFile -notin @('client.pid','server.pid')) { throw 'Unowned Android cleanup target' }
    return 'if [ -f '+$Directory+'/'+$PidFile+' ]; then p=$(cat '+$Directory+'/'+$PidFile+'); case "$p" in ""|*[!0-9]*) exit 1;; esac; if [ -r /proc/$p/cmdline ]; then c=$(tr "\000" " " < /proc/$p/cmdline); case "$c" in "'+$Directory+'/bench "*) kill "$p"; for w in 1 2 3 4 5; do [ ! -d /proc/$p ] && break; sleep 1; done; [ ! -d /proc/$p ] || exit 1;; esac; fi; fi; rm -rf '+$Directory+'; [ ! -e '+$Directory+' ]'
}
