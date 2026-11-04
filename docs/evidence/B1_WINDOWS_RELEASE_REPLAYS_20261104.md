# B1 Windows release replays — 2026-11-04

Stage 1 blocker B1 of the 2026-11 completion roadmap. These are the release
checks that the Linux cloud could not run, now run on this Windows laptop
(PowerShell 7.6.9, Python 3.14.3) at branch `task/s0-rebaseline-20261103`. No
MSI, app, device or network step ran.

| Replay | Result |
| --- | --- |
| `python -m unittest discover -s tools/release -p "test_*.py"` (assembler and MSI backend, 83 methods) | 74 passed, 9 skipped, 0 failed |
| `pwsh -File tools/release/test_msi_wrapper.ps1` (requires PowerShell 7.2 or later) | 16 passed; two owned PowerShell children |
| `python tools/release/probe_windows_metadata.py --work <fresh temp dir>` | completed; native open/closed/path/handle snapshots agree; Python path `st_ctime_ns` reports creation time while the handle reports change time, which the native snapshot keeps distinct |

The Windows-only methods that the cloud skipped now execute and pass. These are
the `test_assemble.WindowsMetadata` methods and the MSI host-identity witnesses.
The 9 skips are POSIX-only symlink, hardlink, FIFO and descriptor fixtures,
which do not apply on Windows. Actual finalized app-image, WiX/jpackage
admission, MSI construction and install/uninstall remain Stage 5 (T5.06) work.
