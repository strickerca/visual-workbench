"""HOOK-002: fail closed on platform imports in shared commonMain."""
from pathlib import Path
import argparse
import re


def uncomment(text: str) -> str:
    out, i, depth = [], 0, 0
    quote = None
    while i < len(text):
        if quote:
            if quote != '"""' and text[i] == '\\':
                out.extend('  '); i += 2
            elif text.startswith(quote, i):
                out.extend(' ' * len(quote)); i += len(quote); quote = None
            else:
                out.append('\n' if text[i] == '\n' else ' '); i += 1
        elif depth and text.startswith('/*', i):
            depth += 1; i += 2
        elif depth and text.startswith('*/', i):
            depth -= 1; i += 2; out.append(' ')
        elif depth:
            if text[i] == '\n': out.append('\n')
            i += 1
        elif text.startswith('//', i):
            end = text.find('\n', i)
            i = len(text) if end < 0 else end
        elif text.startswith('/*', i):
            depth = 1; i += 2
        elif text.startswith('"""', i):
            quote = '"""'; out.extend('   '); i += 3
        elif text[i] in {'"', "'"}:
            quote = text[i]; out.append(' '); i += 1
        else:
            out.append(text[i]); i += 1
    return ''.join(out)


def violations(text: str) -> list[str]:
    # No AndroidX artifacts are allowlisted yet. Add only exact reviewed KMP
    # package prefixes with a dependency record when common code needs them.
    result = []
    for match in re.finditer(r'^\s*import\s+([^;\n]+)', uncomment(text), re.M):
        name = re.sub(r'\s+', '', match.group(1).split(' as ')[0]).replace('`', '')
        if name.split('.')[0] in {'android', 'androidx', 'java', 'javax'}:
            result.append(name)
    return result


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument('root', type=Path)
    args = parser.parse_args()
    if not args.root.is_dir(): parser.error('commonMain directory missing')
    failures = [(path.relative_to(args.root), value) for path in sorted(args.root.rglob('*.kt')) for value in violations(path.read_text(encoding='utf-8'))]
    for path, value in failures: print(f'{path}: forbidden commonMain import {value}')
    if failures: raise SystemExit(1)
    print('HOOK-002 commonMain platform imports: PASS')


if __name__ == '__main__': main()
