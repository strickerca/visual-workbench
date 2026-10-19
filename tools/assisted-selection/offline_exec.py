#!/usr/bin/env python3
"""Exec a command with an inherited kernel syscall filter denying network IO."""
import ctypes
import errno
import os
import resource
import sys

if len(sys.argv) < 2:
    raise SystemExit('usage: offline_exec.py COMMAND [ARGS...]')
lib = ctypes.CDLL('libseccomp.so.2', use_errno=True)
lib.seccomp_init.argtypes = [ctypes.c_uint32]
lib.seccomp_init.restype = ctypes.c_void_p
lib.seccomp_syscall_resolve_name.argtypes = [ctypes.c_char_p]
lib.seccomp_syscall_resolve_name.restype = ctypes.c_int
lib.seccomp_rule_add.argtypes = [ctypes.c_void_p, ctypes.c_uint32, ctypes.c_int, ctypes.c_uint]
lib.seccomp_rule_add.restype = ctypes.c_int
lib.seccomp_load.argtypes = [ctypes.c_void_p]
lib.seccomp_load.restype = ctypes.c_int
lib.seccomp_release.argtypes = [ctypes.c_void_p]
context = lib.seccomp_init(0x7fff0000)  # SCMP_ACT_ALLOW
if not context:
    raise SystemExit('seccomp_init failed')
try:
    denied = ('socket', 'socketpair', 'connect', 'bind', 'listen', 'accept', 'accept4',
              'sendto', 'sendmsg', 'sendmmsg', 'recvfrom', 'recvmsg', 'recvmmsg',
              'io_uring_setup')
    for name in denied:
        number = lib.seccomp_syscall_resolve_name(name.encode('ascii'))
        if number < 0:
            raise SystemExit('missing syscall: ' + name)
        if lib.seccomp_rule_add(context, 0x00050000 | errno.EPERM, number, 0) != 0:
            raise SystemExit('failed to deny syscall: ' + name)
    if lib.seccomp_load(context) != 0:
        raise SystemExit('seccomp_load failed')
finally:
    lib.seccomp_release(context)
resource.setrlimit(resource.RLIMIT_AS, (10 * 1024**3, 10 * 1024**3))
resource.setrlimit(resource.RLIMIT_FSIZE, (512 * 1024**2, 512 * 1024**2))
os.execvpe(sys.argv[1], sys.argv[1:], os.environ)
