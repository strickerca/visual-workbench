#!/usr/bin/env bash
# Type-checks and lints the whole Rust workspace for Windows from a Linux
# machine: the clippy command of tools/ci/windows-rust.ps1 with
# --target x86_64-pc-windows-msvc. It reaches the Windows-only code (capture,
# input injection, UI Automation, the PC host) that a Linux build skips.
# Nothing is linked or run, so a pass means "compiles and lints for Windows",
# not "works on Windows"; the Windows job stays the real check.
#
# Needs the x86_64-pc-windows-msvc Rust target and LLVM's clang-cl and
# llvm-lib. C code in build scripts is not really compiled: a stand-in
# compiler answers the compiler-family probes with the real clang-cl and
# writes an empty object for every compile, which is enough because clippy
# never links.
. "$(dirname "${BASH_SOURCE[0]}")/common.sh"
target=x86_64-pc-windows-msvc

rustup target list --installed | grep -qx "$target" ||
  { echo "the Rust target $target is missing: rustup target add $target" >&2; exit 1; }
real="$(command -v clang-cl || compgen -c clang-cl- | sort -V | tail -n 1 || true)"
[ -n "$real" ] && command -v "$real" >/dev/null ||
  { echo "clang-cl (LLVM) is missing" >&2; exit 1; }
command -v llvm-lib >/dev/null || { echo "llvm-lib (LLVM) is missing" >&2; exit 1; }

stand_in="$(mktemp -d)"
trap 'rm -rf "$stand_in"' EXIT
printf 'typedef int vw_empty_unit;\n' > "$stand_in/empty.c"
cat > "$stand_in/clang-cl" <<EOF
#!/usr/bin/env bash
out=""; probe=0
for a in "\$@"; do
  case "\$a" in
    -E|/E|-EP|/EP|-P|/P) probe=1 ;;
    -Fo*|/Fo*) out="\${a:3}" ;;
  esac
done
[ "\$probe" = 1 ] && exec "$real" --target=$target "\$@"
if [ -n "\$out" ]; then
  case "\$out" in */) out="\${out}empty.obj" ;; esac
  exec "$real" --target=$target -c "$stand_in/empty.c" "-Fo\$out"
fi
"$real" --target=$target "\$@" 2>/dev/null
exit 0
EOF
chmod +x "$stand_in/clang-cl"

step "clippy for Windows ($target, not linked)"
CC_x86_64_pc_windows_msvc="$stand_in/clang-cl" CXX_x86_64_pc_windows_msvc="$stand_in/clang-cl" \
AR_x86_64_pc_windows_msvc=llvm-lib \
  cargo clippy --workspace --all-targets --locked --target "$target" -- -D warnings
