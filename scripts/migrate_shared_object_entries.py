#!/usr/bin/env python3
from pathlib import Path
import re


def migrate(path: str) -> None:
    file = Path(path)
    lines = file.read_text().splitlines()
    index = 0
    migrated = 0
    while index < len(lines):
        match = re.search(
            r'let\s+(\w+)\s*:\s*&mut\s+(.+?)\s*=\s*object::\w+<.+>\((\w+)\);',
            lines[index].strip(),
        )
        if not match:
            index += 1
            continue
        object_name, object_type, id_name = match.groups()
        function_start = index
        while function_start >= 0 and 'public entry fun ' not in lines[function_start]:
            function_start -= 1
        if function_start < 0:
            raise SystemExit(f'{path}: entry function missing for {object_name}')
        parameter_line = None
        for cursor in range(function_start, index):
            if re.search(rf'\b{re.escape(id_name)}\s*:\s*address\b', lines[cursor]):
                parameter_line = cursor
                break
        if parameter_line is None:
            raise SystemExit(f'{path}: parameter {id_name} missing')
        indentation = lines[parameter_line][: len(lines[parameter_line]) - len(lines[parameter_line].lstrip())]
        suffix = ',' if lines[parameter_line].rstrip().endswith(',') else ''
        lines[parameter_line] = f'{indentation}{object_name}: &mut {object_type}{suffix}'
        del lines[index]
        migrated += 1
    if migrated == 0:
        raise SystemExit(f'{path}: no object lookups migrated')
    file.write_text('\n'.join(lines) + '\n')


migrate('sdk/kanari_pay/backend/dex_v1/sources/dex_v1.move')
migrate('sdk/kanari_pay/backend/kanari_escrow/sources/escrow.move')

# Persist long-lived application objects through the shared transfer path.
for path, replacements in {
    'sdk/kanari_pay/backend/dex_v1/sources/dex_v1.move': [
        ('        object::save_object(&pool);\n\n', ''),
        ('        transfer::public_transfer(pool, tx_context::sender(ctx));', '        transfer::share_object(pool);'),
    ],
    'sdk/kanari_pay/backend/kanari_escrow/sources/escrow.move': [
        ('        transfer::public_transfer(deal, buyer_addr);', '        transfer::share_object(deal);'),
        ('        transfer::public_transfer(proof, buyer_addr);', '        transfer::share_object(proof);'),
    ],
}.items():
    file = Path(path)
    text = file.read_text()
    for old, new in replacements:
        if text.count(old) != 1:
            raise SystemExit(f'{path}: shared transfer anchor mismatch')
        text = text.replace(old, new)
    file.write_text(text)
