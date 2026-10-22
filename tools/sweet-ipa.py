#!/usr/bin/env python3
"""IPA & App Bundle Inspector, Installer, and TrustCache Toolkit for ios-sweet.

Standalone Python 3 utility using only the Python standard library (works on
Windows, Linux, and macOS). Provides both a CLI interface and importable Python
functions for `tools/sweet-emu.py` and root image preparation scripts.

CLI Usage:
  python tools/sweet-ipa.py info <path/to/app.ipa_or_dir.app> [--json]
  python tools/sweet-ipa.py install <path/to/app.ipa_or_dir.app> --root <root_dir> [--trustcache <out.tc>]
  python tools/sweet-ipa.py list --root <root_dir> [--json]
  python tools/sweet-ipa.py uninstall <bundle_id> --root <root_dir>
  python tools/sweet-ipa.py create-demo-ipa <output.ipa>
"""

import argparse
import datetime
import hashlib
import io
import json
import os
import plistlib
import shutil
import stat
import struct
import sys
import tempfile
import uuid
import zipfile
from typing import Any, Dict, List, Optional, Sequence, Tuple, Union

# ---------------------------------------------------------------------------
# Mach-O & Code Signing Constants
# ---------------------------------------------------------------------------

MH_MAGIC = 0xFEEDFACE
MH_CIGAM = 0xCEFAEDFE
MH_MAGIC_64 = 0xFEEDFACF
MH_CIGAM_64 = 0xCFFAEDFE
FAT_MAGIC = 0xCAFEBABE
FAT_CIGAM = 0xBEBAFECA
FAT_MAGIC_64 = 0xCAFEBABF
FAT_CIGAM_64 = 0xBFBAFECA

MACHO_MAGICS = {
    MH_MAGIC,
    MH_CIGAM,
    MH_MAGIC_64,
    MH_CIGAM_64,
    FAT_MAGIC,
    FAT_CIGAM,
    FAT_MAGIC_64,
    FAT_CIGAM_64,
}

CPU_TYPE_ARM = 0x0000000C
CPU_TYPE_ARM64 = 0x0100000C
CPU_TYPE_ARM64_32 = 0x0200000C
CPU_TYPE_X86_64 = 0x01000007

MH_OBJECT = 0x1
MH_EXECUTE = 0x2
MH_FVMLIB = 0x3
MH_CORE = 0x4
MH_PRELOAD = 0x5
MH_DYLIB = 0x6
MH_DYLINKER = 0x7
MH_BUNDLE = 0x8

MH_NOUNDEFS = 0x1
MH_DYLDLINK = 0x4
MH_TWOLEVEL = 0x80
MH_PIE = 0x200000

LC_SEGMENT = 0x01
LC_SYMTAB = 0x02
LC_UNIXTHREAD = 0x05
LC_LOAD_DYLIB = 0x0C
LC_ID_DYLIB = 0x0D
LC_LOAD_DYLINKER = 0x0E
LC_SEGMENT_64 = 0x19
LC_UUID = 0x1B
LC_RPATH = 0x8000001C
LC_CODE_SIGNATURE = 0x1D
LC_SEGMENT_SPLIT_INFO = 0x1E
LC_REEXPORT_DYLIB = 0x8000001F
LC_LAZY_LOAD_DYLIB = 0x20
LC_ENCRYPTION_INFO = 0x21
LC_LOAD_UPWARD_DYLIB = 0x80000023
LC_VERSION_MIN_IPHONEOS = 0x25
LC_MAIN = 0x80000028
LC_ENCRYPTION_INFO_64 = 0x2C
LC_BUILD_VERSION = 0x32
LC_WEAK_DYLIB = 0x80000018

DYLIB_LOAD_COMMANDS = {
    LC_LOAD_DYLIB,
    LC_WEAK_DYLIB,
    LC_REEXPORT_DYLIB,
    LC_LAZY_LOAD_DYLIB,
    LC_LOAD_UPWARD_DYLIB,
}

# Code Signing blobs
CSMAGIC_REQUIREMENT = 0xFADE0C00
CSMAGIC_REQUIREMENTS = 0xFADE0C01
CSMAGIC_CODEDIRECTORY = 0xFADE0C02
CSMAGIC_EMBEDDED_SIGNATURE = 0xFADE0CC0
CSMAGIC_DETACHED_SIGNATURE = 0xFADE0CC1
CSMAGIC_BLOBWRAPPER = 0xFADE0B01
CSMAGIC_EMBEDDED_ENTITLEMENTS = 0xFADE7171
CSMAGIC_EMBEDDED_DER_ENTITLEMENTS = 0xFADE7172

CSSLOT_CODEDIRECTORY = 0
CSSLOT_INFOSLOT = 1
CSSLOT_REQUIREMENTS = 2
CSSLOT_RESOURCEDIR = 3
CSSLOT_APPLICATION = 4
CSSLOT_ENTITLEMENTS = 5
CSSLOT_DER_ENTITLEMENTS = 7
CSSLOT_ALTERNATE_CODEDIRECTORIES = (0x1000, 0x1001, 0x1002, 0x1003, 0x1004)
CSSLOT_SIGNATURESLOT = 0x10000

CS_ADHOC = 0x00000002
CS_HASHTYPE_SHA1 = 1
CS_HASHTYPE_SHA256 = 2
CS_HASHTYPE_SHA256_TRUNCATED = 3
CS_HASHTYPE_SHA384 = 4

CS_CDHASH_LEN = 20
TRUSTCACHE_VERSION = 1

# iOS UIDs/GIDs
UID_ROOT = 0
UID_INSTALLD = 33
UID_MOBILE = 501

# Known graphics & game frameworks of interest
KNOWN_FRAMEWORKS = {
    'Metal.framework': 'Metal',
    'MetalKit.framework': 'MetalKit',
    'OpenGLES.framework': 'OpenGLES',
    'GLKit.framework': 'GLKit',
    'SpriteKit.framework': 'SpriteKit',
    'SceneKit.framework': 'SceneKit',
    'UnityFramework.framework': 'UnityFramework',
    'UnityFramework': 'UnityFramework',
    'UIKit.framework': 'UIKit',
    'QuartzCore.framework': 'QuartzCore',
    'CoreGraphics.framework': 'CoreGraphics',
    'GameController.framework': 'GameController',
    'GameKit.framework': 'GameKit',
    'AVFoundation.framework': 'AVFoundation',
    'AudioToolbox.framework': 'AudioToolbox',
    'OpenAL.framework': 'OpenAL',
    'CoreMotion.framework': 'CoreMotion',
    'WebKit.framework': 'WebKit',
}

CAPABILITY_KEYS_OF_INTEREST = (
    'arm64',
    'armv7',
    'metal',
    'opengles-1',
    'opengles-2',
    'opengles-3',
    'gamekit',
    'accelerometer',
    'gyroscope',
    'wifi',
    'bluetooth-le',
)

# Deterministic namespace UUID for ios-sweet trust cache
SWEET_TRUSTCACHE_UUID = uuid.UUID('53574545-5454-4331-8000-000000000001').bytes


# ---------------------------------------------------------------------------
# Filesystem & Permission Helpers
# ---------------------------------------------------------------------------

def safe_chown(path: str, uid: int, gid: int) -> None:
    """Set file/directory owner on POSIX systems; gracefully skip on Windows or unprivileged runs."""
    chown_fn = getattr(os, 'chown', None)
    if chown_fn is None:
        return
    try:
        chown_fn(path, uid, gid)
    except (PermissionError, OSError):
        pass


def safe_chmod(path: str, mode: int) -> None:
    """Set file/directory permissions; ignore errors on filesystems that do not support POSIX modes."""
    try:
        os.chmod(path, mode)
    except (PermissionError, OSError):
        pass


def apply_tree_permissions(root_path: str, uid: int, gid: int, exec_rel_paths: Optional[set] = None) -> None:
    """Recursively apply ownership and permissions to a directory tree."""
    exec_rel_paths = exec_rel_paths or set()
    safe_chown(root_path, uid, gid)
    safe_chmod(root_path, 0o755)
    for dirpath, dirnames, filenames in os.walk(root_path):
        for d in dirnames:
            dp = os.path.join(dirpath, d)
            safe_chown(dp, uid, gid)
            safe_chmod(dp, 0o755)
        for fname in filenames:
            fp = os.path.join(dirpath, fname)
            rel = os.path.relpath(fp, root_path).replace('\\', '/')
            safe_chown(fp, uid, gid)
            if rel in exec_rel_paths or fname.endswith('.dylib') or fname.endswith('.so'):
                safe_chmod(fp, 0o755)
            else:
                safe_chmod(fp, 0o644)


def _handle_remove_readonly(func, path, exc_info):
    """Error handler for shutil.rmtree on Windows when read-only files are encountered."""
    safe_chmod(path, stat.S_IWRITE | stat.S_IREAD)
    func(path)


def safe_rmtree(path: str) -> None:
    """Remove a directory tree, handling read-only attributes on Windows."""
    if os.path.exists(path):
        shutil.rmtree(path, onerror=_handle_remove_readonly)


def save_plist(path: str, data: Any, uid: int = UID_MOBILE, gid: int = UID_MOBILE,
               mode: int = 0o644, fmt=plistlib.FMT_BINARY) -> None:
    """Write a plist file atomically and set ownership/permissions."""
    os.makedirs(os.path.dirname(os.path.abspath(path)), exist_ok=True)
    with open(path, 'wb') as f:
        plistlib.dump(data, f, fmt=fmt)
    safe_chown(path, uid, gid)
    safe_chmod(path, mode)


def load_plist_bytes(raw: bytes) -> Dict[str, Any]:
    """Parse XML or binary plist bytes into a dictionary."""
    obj = plistlib.loads(raw)
    if isinstance(obj, dict):
        return obj
    return {'_root': obj}


def load_plist_file(path: str) -> Dict[str, Any]:
    """Read an XML or binary plist file from disk."""
    with open(path, 'rb') as f:
        return load_plist_bytes(f.read())


# ---------------------------------------------------------------------------
# Mach-O & CodeSignature Parser
# ---------------------------------------------------------------------------

def _detect_fat_endian(data: bytes) -> Tuple[Optional[str], bool]:
    """Return (endian, is_fat64) if `data` starts with a Fat Mach-O header, else (None, False)."""
    if len(data) < 8:
        return None, False
    magic_le, = struct.unpack_from('<I', data, 0)
    if magic_le not in (FAT_MAGIC, FAT_CIGAM, FAT_MAGIC_64, FAT_CIGAM_64):
        return None, False
    is_fat64 = magic_le in (FAT_MAGIC_64, FAT_CIGAM_64)
    n_be, = struct.unpack_from('>I', data, 4)
    if 1 <= n_be <= 32:
        return '>', is_fat64
    n_le, = struct.unpack_from('<I', data, 4)
    if 1 <= n_le <= 32:
        return '<', is_fat64
    return None, False


def is_macho_bytes(data: bytes) -> bool:
    """Return True if the first 4 bytes look like a valid Mach-O or Universal (Fat) binary."""
    if len(data) < 28:
        return False
    magic, = struct.unpack_from('<I', data, 0)
    if magic in (MH_MAGIC, MH_CIGAM, MH_MAGIC_64, MH_CIGAM_64):
        return True
    fat_endian, _ = _detect_fat_endian(data)
    return fat_endian is not None


def _hash_name(hash_type: int) -> str:
    return {
        CS_HASHTYPE_SHA1: 'sha1',
        CS_HASHTYPE_SHA256: 'sha256',
        CS_HASHTYPE_SHA256_TRUNCATED: 'sha256_truncated',
        CS_HASHTYPE_SHA384: 'sha384',
    }.get(hash_type, f'unknown({hash_type})')


def _compute_cd_digest(cd_blob: bytes, hash_type: int) -> bytes:
    if hash_type == CS_HASHTYPE_SHA1:
        return hashlib.sha1(cd_blob).digest()
    if hash_type == CS_HASHTYPE_SHA384:
        return hashlib.sha384(cd_blob).digest()
    return hashlib.sha256(cd_blob).digest()


def parse_code_signature(sig_data: bytes) -> Dict[str, Any]:
    """Parse an LC_CODE_SIGNATURE SuperBlob and extract CodeDirectory CDHashes & entitlements."""
    result: Dict[str, Any] = {
        'present': False,
        'adhoc': False,
        'identifier': None,
        'team_id': None,
        'hash_type': None,
        'hash_type_id': None,
        'page_size': None,
        'flags': 0,
        'cdhash': None,         # 20-byte hex (40 chars) for TrustCache
        'cdhash_full': None,    # Full digest hex (64 chars for SHA-256)
        'all_cdhashes': [],
        'entitlements': None,
    }
    if len(sig_data) < 12:
        return result

    magic, total_len, count = struct.unpack_from('>3I', sig_data, 0)
    if magic != CSMAGIC_EMBEDDED_SIGNATURE:
        return result

    result['present'] = True
    best_cd: Optional[Dict[str, Any]] = None

    for i in range(count):
        idx_off = 12 + i * 8
        if idx_off + 8 > len(sig_data):
            break
        slot_type, blob_off = struct.unpack_from('>II', sig_data, idx_off)
        if blob_off + 8 > len(sig_data):
            continue
        b_magic, b_len = struct.unpack_from('>II', sig_data, blob_off)
        blob_end = min(blob_off + b_len, len(sig_data))
        blob = sig_data[blob_off:blob_end]

        if b_magic == CSMAGIC_CODEDIRECTORY and len(blob) >= 44:
            version, flags, hash_off, ident_off, n_special, n_code, code_limit = struct.unpack_from(
                '>7I', blob, 8
            )
            hash_size, hash_type, platform, page_shift = struct.unpack_from('>4B', blob, 36)
            ident = None
            if 0 < ident_off < len(blob):
                ident_bytes = blob[ident_off:].split(b'\x00', 1)[0]
                ident = ident_bytes.decode('utf-8', errors='replace')

            team_id = None
            if version >= 0x20200 and len(blob) >= 52:
                team_off, = struct.unpack_from('>I', blob, 48)
                if 0 < team_off < len(blob):
                    team_bytes = blob[team_off:].split(b'\x00', 1)[0]
                    team_id = team_bytes.decode('utf-8', errors='replace')

            digest = _compute_cd_digest(blob, hash_type)
            cdhash_20 = digest[:CS_CDHASH_LEN].hex()
            cdhash_full = digest.hex()

            cd_info = {
                'slot_type': slot_type,
                'version': version,
                'flags': flags,
                'adhoc': bool(flags & CS_ADHOC),
                'identifier': ident,
                'team_id': team_id,
                'hash_type_id': hash_type,
                'hash_type': _hash_name(hash_type),
                'hash_size': hash_size,
                'page_size': (1 << page_shift) if page_shift else 0,
                'n_special_slots': n_special,
                'n_code_slots': n_code,
                'code_limit': code_limit,
                'cdhash': cdhash_20,
                'cdhash_full': cdhash_full,
            }
            result['all_cdhashes'].append(cd_info)

            # Prefer SHA-256 (hash_type == 2) when multiple CodeDirectories exist
            if best_cd is None or (hash_type == CS_HASHTYPE_SHA256 and best_cd['hash_type_id'] != CS_HASHTYPE_SHA256):
                best_cd = cd_info

        elif b_magic == CSMAGIC_EMBEDDED_ENTITLEMENTS and len(blob) > 8:
            xml_bytes = blob[8:].rstrip(b'\x00')
            try:
                result['entitlements'] = load_plist_bytes(xml_bytes)
            except Exception:
                result['entitlements'] = {'_raw': xml_bytes.decode('utf-8', errors='replace')}

    if best_cd is not None:
        result['adhoc'] = best_cd['adhoc']
        result['identifier'] = best_cd['identifier']
        result['team_id'] = best_cd['team_id']
        result['hash_type'] = best_cd['hash_type']
        result['hash_type_id'] = best_cd['hash_type_id']
        result['page_size'] = best_cd['page_size']
        result['flags'] = best_cd['flags']
        result['cdhash'] = best_cd['cdhash']
        result['cdhash_full'] = best_cd['cdhash_full']

    return result


def _parse_single_macho_slice(slice_data: bytes, rel_path: str = '') -> Dict[str, Any]:
    """Parse a single-architecture Mach-O slice."""
    info: Dict[str, Any] = {
        'path': rel_path,
        'valid': False,
        'is_64bit': False,
        'cpu_type': None,
        'cpu_subtype': None,
        'is_arm64': False,
        'filetype': None,
        'flags': 0,
        'uuid': None,
        'encrypted': False,
        'cryptid': 0,
        'cryptoff': 0,
        'cryptsize': 0,
        'dylibs': [],
        'frameworks': [],
        'rpaths': [],
        'has_main': False,
        'entryoff': None,
        'code_signature': None,
        'signed': False,
        'cdhash': None,
        'cdhash_full': None,
        'cdhash_source': None,
    }
    if len(slice_data) < 28:
        return info

    magic, = struct.unpack_from('<I', slice_data, 0)
    if magic == MH_MAGIC_64:
        endian = '<'
        is_64 = True
        hdr_size = 32
    elif magic == MH_CIGAM_64:
        endian = '>'
        is_64 = True
        hdr_size = 32
    elif magic == MH_MAGIC:
        endian = '<'
        is_64 = False
        hdr_size = 28
    elif magic == MH_CIGAM:
        endian = '>'
        is_64 = False
        hdr_size = 28
    else:
        return info

    if len(slice_data) < hdr_size:
        return info

    _, cpu_type, cpu_subtype, filetype, ncmds, sizeofcmds, flags = struct.unpack_from(
        f'{endian}7I', slice_data, 0
    )

    info['valid'] = True
    info['is_64bit'] = is_64
    info['cpu_type'] = cpu_type
    info['cpu_subtype'] = cpu_subtype & 0x00FFFFFF
    info['is_arm64'] = (cpu_type == CPU_TYPE_ARM64)
    info['filetype'] = filetype
    info['flags'] = flags

    off = hdr_size
    max_off = min(len(slice_data), hdr_size + sizeofcmds)

    sig_blob: Optional[bytes] = None

    for _ in range(ncmds):
        if off + 8 > max_off:
            break
        cmd, cmdsize = struct.unpack_from(f'{endian}II', slice_data, off)
        if cmdsize < 8 or off + cmdsize > len(slice_data):
            break

        if cmd in DYLIB_LOAD_COMMANDS and cmdsize >= 24:
            str_off, = struct.unpack_from(f'{endian}I', slice_data, off + 8)
            if 8 <= str_off < cmdsize:
                raw_str = slice_data[off + str_off:off + cmdsize].split(b'\x00', 1)[0]
                dylib_path = raw_str.decode('utf-8', errors='replace')
                info['dylibs'].append(dylib_path)
                for fw_marker, fw_name in KNOWN_FRAMEWORKS.items():
                    if fw_marker in dylib_path and fw_name not in info['frameworks']:
                        info['frameworks'].append(fw_name)

        elif cmd == LC_RPATH and cmdsize >= 12:
            str_off, = struct.unpack_from(f'{endian}I', slice_data, off + 8)
            if 8 <= str_off < cmdsize:
                raw_str = slice_data[off + str_off:off + cmdsize].split(b'\x00', 1)[0]
                info['rpaths'].append(raw_str.decode('utf-8', errors='replace'))

        elif cmd in (LC_ENCRYPTION_INFO_64, LC_ENCRYPTION_INFO) and cmdsize >= 20:
            cryptoff, cryptsize, cryptid = struct.unpack_from(f'{endian}3I', slice_data, off + 8)
            info['cryptoff'] = cryptoff
            info['cryptsize'] = cryptsize
            info['cryptid'] = cryptid
            info['encrypted'] = (cryptid != 0)

        elif cmd == LC_UUID and cmdsize >= 24:
            info['uuid'] = str(uuid.UUID(bytes=slice_data[off + 8:off + 24])).upper()

        elif cmd == LC_MAIN and cmdsize >= 24:
            entryoff, _ = struct.unpack_from(f'{endian}QQ', slice_data, off + 8)
            info['has_main'] = True
            info['entryoff'] = entryoff

        elif cmd == LC_UNIXTHREAD:
            info['has_main'] = True

        elif cmd == LC_CODE_SIGNATURE and cmdsize >= 16:
            dataoff, datasize = struct.unpack_from(f'{endian}II', slice_data, off + 8)
            if dataoff + datasize <= len(slice_data):
                sig_blob = slice_data[dataoff:dataoff + datasize]

        off += cmdsize

    if sig_blob is not None:
        cs_info = parse_code_signature(sig_blob)
        info['code_signature'] = cs_info
        if cs_info.get('cdhash'):
            info['signed'] = True
            info['cdhash'] = cs_info['cdhash']
            info['cdhash_full'] = cs_info['cdhash_full']
            info['cdhash_source'] = 'LC_CODE_SIGNATURE'

    if not info['cdhash']:
        # Compute ad-hoc SHA-256 digest over the Mach-O slice if unsigned
        digest = hashlib.sha256(slice_data).digest()
        info['cdhash'] = digest[:CS_CDHASH_LEN].hex()
        info['cdhash_full'] = digest.hex()
        info['cdhash_source'] = 'adhoc_sha256'

    return info


def parse_macho_file(data: bytes, rel_path: str = '') -> Dict[str, Any]:
    """Parse a thin or universal (Fat) Mach-O binary, preferring the ARM64 slice."""
    if not is_macho_bytes(data):
        return {'path': rel_path, 'valid': False, 'is_arm64': False}

    fat_endian, is_fat64 = _detect_fat_endian(data)
    if fat_endian is not None:
        endian = fat_endian
        nfat_arch, = struct.unpack_from(f'{endian}I', data, 4)
        slices = []
        arch_list = []
        off = 8
        for _ in range(nfat_arch):
            if is_fat64:
                if off + 32 > len(data):
                    break
                cputype, cpusubtype, slice_off, slice_size, _ = struct.unpack_from(
                    f'{endian}IIQQI', data, off
                )
                off += 32
            else:
                if off + 20 > len(data):
                    break
                cputype, cpusubtype, slice_off, slice_size, _ = struct.unpack_from(
                    f'{endian}5I', data, off
                )
                off += 20

            arch_list.append(cputype)
            if slice_off + slice_size <= len(data):
                slices.append((cputype, data[slice_off:slice_off + slice_size]))

        # Prefer CPU_TYPE_ARM64 slice
        chosen_slice = None
        for cputype, sdata in slices:
            if cputype == CPU_TYPE_ARM64:
                chosen_slice = sdata
                break
        if chosen_slice is None and slices:
            chosen_slice = slices[0][1]

        if chosen_slice is None:
            return {'path': rel_path, 'valid': False, 'is_arm64': False}

        parsed = _parse_single_macho_slice(chosen_slice, rel_path)
        parsed['is_fat'] = True
        parsed['fat_cpu_types'] = arch_list
        parsed['is_arm64'] = (CPU_TYPE_ARM64 in arch_list)
        return parsed

    parsed = _parse_single_macho_slice(data, rel_path)
    parsed['is_fat'] = False
    parsed['fat_cpu_types'] = [parsed['cpu_type']] if parsed['cpu_type'] is not None else []
    return parsed


# ---------------------------------------------------------------------------
# IPA / .app Bundle Inspection
# ---------------------------------------------------------------------------

def _normalize_capabilities(caps_raw: Any) -> Dict[str, bool]:
    """Normalize UIRequiredDeviceCapabilities (list or dict) into a dict of booleans."""
    normalized: Dict[str, bool] = {}
    if isinstance(caps_raw, list):
        for item in caps_raw:
            if isinstance(item, str):
                normalized[item] = True
    elif isinstance(caps_raw, dict):
        for k, v in caps_raw.items():
            normalized[str(k)] = bool(v)
    return normalized


def _classify_graphics_api(frameworks: List[str], capabilities: Dict[str, bool]) -> Dict[str, Any]:
    """Classify the app's rendering & game engine profile."""
    uses_metal = 'Metal' in frameworks or 'MetalKit' in frameworks or capabilities.get('metal', False)
    uses_gles = (
        'OpenGLES' in frameworks
        or 'GLKit' in frameworks
        or any(capabilities.get(k, False) for k in ('opengles-1', 'opengles-2', 'opengles-3'))
    )
    uses_spritekit = 'SpriteKit' in frameworks
    uses_scenekit = 'SceneKit' in frameworks
    uses_unity = 'UnityFramework' in frameworks
    uses_uikit = 'UIKit' in frameworks or 'QuartzCore' in frameworks or 'CoreGraphics' in frameworks

    apis: List[str] = []
    if uses_unity:
        apis.append('Unity')
    if uses_spritekit:
        apis.append('SpriteKit')
    if uses_scenekit:
        apis.append('SceneKit')
    if uses_gles:
        apis.append('OpenGLES')
    if uses_metal:
        apis.append('Metal')
    if uses_uikit or not apis:
        apis.append('UIKit/2D')

    if uses_gles and not (uses_metal and not uses_uikit):
        primary = 'OpenGL ES / 2D' if uses_uikit else 'OpenGL ES'
    elif uses_metal and not uses_gles:
        primary = 'Metal + UIKit' if uses_uikit else 'Metal'
    elif uses_metal and uses_gles:
        primary = 'OpenGL ES + Metal'
    else:
        primary = 'UIKit / CoreAnimation 2D'

    return {
        'primary_api': primary,
        'apis': apis,
        'uses_metal': uses_metal,
        'uses_opengles': uses_gles,
        'uses_uikit_2d': uses_uikit,
        'uses_spritekit': uses_spritekit,
        'uses_scenekit': uses_scenekit,
        'uses_unity': uses_unity,
    }


def _iter_bundle_files_from_dir(app_dir: str):
    """Yield (rel_path_inside_app, file_bytes_loader) for all regular files in a .app directory."""
    for root, _, files in os.walk(app_dir):
        for fname in sorted(files):
            full = os.path.join(root, fname)
            if os.path.islink(full):
                continue
            rel = os.path.relpath(full, app_dir).replace('\\', '/')

            def _loader(p=full) -> bytes:
                with open(p, 'rb') as f:
                    return f.read()

            yield rel, _loader


def _find_app_in_directory(path: str) -> str:
    """Resolve a path to the `.app` directory (either direct `.app` or `Payload/*.app`)."""
    path = os.path.abspath(path)
    if path.endswith('.app') and os.path.isdir(path):
        return path
    if os.path.isfile(os.path.join(path, 'Info.plist')):
        return path
    payload_dir = os.path.join(path, 'Payload')
    if os.path.isdir(payload_dir):
        for entry in sorted(os.listdir(payload_dir)):
            cand = os.path.join(payload_dir, entry)
            if entry.endswith('.app') and os.path.isdir(cand):
                return cand
    for entry in sorted(os.listdir(path)):
        cand = os.path.join(path, entry)
        if entry.endswith('.app') and os.path.isdir(cand):
            return cand
    raise FileNotFoundError(f'No .app bundle found inside directory: {path}')


def inspect_ipa(path: str) -> Dict[str, Any]:
    """Inspect an `.ipa` archive or `.app` directory and return a full compatibility & Mach-O report."""
    if not os.path.exists(path):
        raise FileNotFoundError(f'Path does not exist: {path}')

    app_folder_name: str = ''
    info_plist_bytes: Optional[bytes] = None
    file_entries: List[Tuple[str, Any]] = []

    if os.path.isdir(path):
        app_dir = _find_app_in_directory(path)
        app_folder_name = os.path.basename(app_dir)
        info_plist_path = os.path.join(app_dir, 'Info.plist')
        if not os.path.isfile(info_plist_path):
            raise FileNotFoundError(f'Info.plist not found in {app_dir}')
        with open(info_plist_path, 'rb') as f:
            info_plist_bytes = f.read()
        file_entries = list(_iter_bundle_files_from_dir(app_dir))
        source_type = 'app_dir'
    elif zipfile.is_zipfile(path):
        source_type = 'ipa_zip'
        zf = zipfile.ZipFile(path, 'r')
        app_prefix = None
        for name in zf.namelist():
            parts = name.strip('/').split('/')
            if len(parts) >= 2 and parts[0] == 'Payload' and parts[1].endswith('.app'):
                app_prefix = f'{parts[0]}/{parts[1]}/'
                app_folder_name = parts[1]
                break
            elif len(parts) >= 1 and parts[0].endswith('.app'):
                app_prefix = f'{parts[0]}/'
                app_folder_name = parts[0]
                break

        if app_prefix is None:
            zf.close()
            raise ValueError(f'No Payload/*.app bundle found in IPA archive: {path}')

        info_member = app_prefix + 'Info.plist'
        if info_member not in zf.namelist():
            # Case-insensitive fallback
            for name in zf.namelist():
                if name.lower() == info_member.lower():
                    info_member = name
                    break
        info_plist_bytes = zf.read(info_member)

        for info_item in zf.infolist():
            if info_item.is_dir() or not info_item.filename.startswith(app_prefix):
                continue
            rel = info_item.filename[len(app_prefix):]
            if not rel:
                continue

            def _zip_loader(z=zf, member=info_item.filename) -> bytes:
                return z.read(member)

            file_entries.append((rel, _zip_loader))
    else:
        raise ValueError(f'Unsupported bundle path (not a directory or .ipa zip): {path}')

    plist = load_plist_bytes(info_plist_bytes)

    bundle_id = str(plist.get('CFBundleIdentifier', ''))
    display_name = str(plist.get('CFBundleDisplayName') or plist.get('CFBundleName') or app_folder_name[:-4])
    bundle_name = str(plist.get('CFBundleName') or display_name)
    short_version = str(plist.get('CFBundleShortVersionString') or plist.get('CFBundleVersion') or '1.0')
    bundle_version = str(plist.get('CFBundleVersion') or short_version)
    executable_name = str(plist.get('CFBundleExecutable') or '')
    min_os_version = str(plist.get('MinimumOSVersion') or plist.get('DTPlatformVersion') or 'Unknown')
    device_family = plist.get('UIDeviceFamily', [])
    if isinstance(device_family, int):
        device_family = [device_family]

    raw_caps = plist.get('UIRequiredDeviceCapabilities', [])
    capabilities = _normalize_capabilities(raw_caps)

    macho_binaries: List[Dict[str, Any]] = []
    main_binary_info: Optional[Dict[str, Any]] = None
    all_frameworks: List[str] = []
    all_dylibs: List[str] = []
    cdhashes: List[str] = []

    for rel, loader in file_entries:
        is_main = (rel == executable_name)
        is_candidate = (
            is_main
            or rel.endswith('.dylib')
            or '.framework/' in rel
            or '.appex/' in rel
            or '/' not in rel
        )
        if not is_candidate:
            continue
        try:
            raw = loader()
        except Exception:
            continue
        if not is_macho_bytes(raw):
            continue

        m_info = parse_macho_file(raw, rel)
        if not m_info.get('valid'):
            continue

        m_info['is_main_executable'] = is_main
        macho_binaries.append(m_info)
        if is_main:
            main_binary_info = m_info

        for fw in m_info.get('frameworks', []):
            if fw not in all_frameworks:
                all_frameworks.append(fw)
        for dy in m_info.get('dylibs', []):
            if dy not in all_dylibs:
                all_dylibs.append(dy)
        cdh = m_info.get('cdhash')
        if cdh and cdh not in cdhashes:
            cdhashes.append(cdh)

    if source_type == 'ipa_zip':
        zf.close()

    if main_binary_info is None and macho_binaries:
        main_binary_info = macho_binaries[0]

    any_encrypted = any(b.get('encrypted', False) for b in macho_binaries)
    all_arm64 = bool(macho_binaries) and all(b.get('is_arm64', False) for b in macho_binaries)
    main_arm64 = bool(main_binary_info and main_binary_info.get('is_arm64', False))
    main_cryptid = int(main_binary_info.get('cryptid', 0)) if main_binary_info else 0

    gfx = _classify_graphics_api(all_frameworks, capabilities)

    warnings: List[str] = []
    if not main_binary_info:
        warnings.append(f'Main executable "{executable_name}" was not found or is not a valid Mach-O binary.')
    elif not main_arm64:
        warnings.append('Main executable does not contain an ARM64 (CPU_TYPE_ARM64 = 0x0100000c) slice.')
    if any_encrypted:
        warnings.append(
            f'Binary is FairPlay-encrypted (cryptid={main_cryptid} != 0). '
            'Decrypt the IPA before running on ios-sweet!'
        )
    if capabilities.get('armv7') and not capabilities.get('arm64') and not main_arm64:
        warnings.append('App requires 32-bit armv7 and lacks an arm64 binary.')

    compatible = bool(main_binary_info and main_arm64 and not any_encrypted)

    entitlements = None
    if main_binary_info and main_binary_info.get('code_signature'):
        entitlements = main_binary_info['code_signature'].get('entitlements')

    return {
        'path': os.path.abspath(path),
        'source_type': source_type,
        'app_folder': app_folder_name,
        'bundle_id': bundle_id,
        'display_name': display_name,
        'bundle_name': bundle_name,
        'short_version': short_version,
        'bundle_version': bundle_version,
        'executable': executable_name,
        'minimum_os_version': min_os_version,
        'device_family': device_family,
        'required_capabilities': capabilities,
        'capabilities_of_interest': {
            k: capabilities.get(k, False) for k in CAPABILITY_KEYS_OF_INTEREST if k in capabilities
        },
        'arm64_supported': main_arm64,
        'all_binaries_arm64': all_arm64,
        'encrypted': any_encrypted,
        'cryptid': main_cryptid,
        'graphics': gfx,
        'frameworks': all_frameworks,
        'dylibs': all_dylibs,
        'cdhashes': cdhashes,
        'entitlements': entitlements,
        'binaries': macho_binaries,
        'compatible': compatible,
        'warnings': warnings,
    }


# Alias for external callers
info_ipa = inspect_ipa
get_ipa_info = inspect_ipa


# ---------------------------------------------------------------------------
# Apple TrustCache v1 Reader & Writer
# ---------------------------------------------------------------------------

def read_trustcache(trustcache_path: str) -> Dict[str, Any]:
    """Read an Apple TrustCache v1 binary file and return its header & entries."""
    if not os.path.isfile(trustcache_path):
        return {
            'version': TRUSTCACHE_VERSION,
            'uuid': str(uuid.UUID(bytes=SWEET_TRUSTCACHE_UUID)).upper(),
            'entries': [],
        }
    with open(trustcache_path, 'rb') as f:
        raw = f.read()
    if len(raw) < 24:
        raise ValueError(f'TrustCache file too small ({len(raw)} bytes): {trustcache_path}')

    version, tc_uuid_bytes, num_entries = struct.unpack_from('<I16sI', raw, 0)
    if version != TRUSTCACHE_VERSION:
        raise ValueError(f'Unsupported TrustCache version {version} in {trustcache_path}')

    entries = []
    off = 24
    for _ in range(num_entries):
        if off + 22 > len(raw):
            break
        cdhash_bytes, hash_type, flags = struct.unpack_from('<20sBB', raw, off)
        entries.append({
            'cdhash': cdhash_bytes.hex(),
            'hash_type': hash_type,
            'flags': flags,
        })
        off += 22

    return {
        'version': version,
        'uuid': str(uuid.UUID(bytes=tc_uuid_bytes)).upper(),
        'num_entries': len(entries),
        'entries': entries,
    }


def write_trustcache(trustcache_path: str, cdhashes: List[Union[str, bytes]],
                     tc_uuid: Optional[bytes] = None) -> Dict[str, Any]:
    """Write a sorted Apple TrustCache v1 binary file (`uint32 version=1, uuid_t uuid, uint32 count` +
    sorted `[20-byte cdhash + uint8 hash_type=2 + uint8 flags=0]`)."""
    tc_uuid_bytes = tc_uuid if (tc_uuid and len(tc_uuid) == 16) else SWEET_TRUSTCACHE_UUID
    unique_hashes: Dict[bytes, Tuple[int, int]] = {}

    for h in cdhashes:
        if isinstance(h, str):
            raw_h = bytes.fromhex(h)[:CS_CDHASH_LEN]
        elif isinstance(h, (bytes, bytearray)):
            raw_h = bytes(h)[:CS_CDHASH_LEN]
        else:
            continue
        if len(raw_h) == CS_CDHASH_LEN:
            unique_hashes[raw_h] = (CS_HASHTYPE_SHA256, 0)

    sorted_keys = sorted(unique_hashes.keys())
    header = struct.pack('<I16sI', TRUSTCACHE_VERSION, tc_uuid_bytes, len(sorted_keys))
    body = b''.join(
        struct.pack('<20sBB', k, unique_hashes[k][0], unique_hashes[k][1])
        for k in sorted_keys
    )

    os.makedirs(os.path.dirname(os.path.abspath(trustcache_path)), exist_ok=True)
    with open(trustcache_path, 'wb') as f:
        f.write(header + body)
    safe_chown(trustcache_path, UID_ROOT, UID_ROOT)
    safe_chmod(trustcache_path, 0o644)

    return {
        'path': os.path.abspath(trustcache_path),
        'version': TRUSTCACHE_VERSION,
        'uuid': str(uuid.UUID(bytes=tc_uuid_bytes)).upper(),
        'num_entries': len(sorted_keys),
        'cdhashes': [k.hex() for k in sorted_keys],
    }


def update_trustcache(trustcache_path: str, new_cdhashes: List[Union[str, bytes]]) -> Dict[str, Any]:
    """Merge new 20-byte CDHashes into an existing TrustCache v1 file (or create a new one)."""
    existing: List[str] = []
    if os.path.isfile(trustcache_path):
        try:
            tc_data = read_trustcache(trustcache_path)
            existing = [e['cdhash'] for e in tc_data.get('entries', [])]
        except Exception:
            existing = []
    combined: List[Union[str, bytes]] = list(existing) + list(new_cdhashes)
    return write_trustcache(trustcache_path, combined)


# ---------------------------------------------------------------------------
# Install / List / Uninstall in Root Image or Staging Directory
# ---------------------------------------------------------------------------

def _registry_plist_path(root_dir: str) -> str:
    return os.path.join(
        root_dir,
        'private', 'var', 'containers', 'Shared', 'SystemGroup',
        'systemgroup.com.apple.installcoordinationd',
        'Library', 'InstallCoordination',
        'sweet_installed_apps.plist',
    )


def _default_trustcache_path(root_dir: str) -> str:
    return os.path.join(root_dir, 'private', 'var', 'db', 'sweet_trustcache.bin')


def _find_existing_container(root_dir: str, subpath: str, bundle_id: str) -> Tuple[Optional[str], Optional[str]]:
    """Find an existing container directory & UUID for `bundle_id` under `private/var/containers/<subpath>`."""
    base = os.path.join(root_dir, 'private', 'var', 'containers', subpath)
    if not os.path.isdir(base):
        return None, None
    for entry in sorted(os.listdir(base)):
        cdir = os.path.join(base, entry)
        meta_path = os.path.join(cdir, '.com.apple.mobile_container_manager.metadata.plist')
        if os.path.isfile(meta_path):
            try:
                meta = load_plist_file(meta_path)
                if meta.get('MCMMetadataIdentifier') == bundle_id:
                    return cdir, str(meta.get('MCMMetadataUUID') or entry)
            except Exception:
                continue
    return None, None


def _extract_to_bundle_dir(src_path: str, dest_app_dir: str) -> None:
    """Extract an `.ipa` or copy a `.app` directory into `dest_app_dir`."""
    safe_rmtree(dest_app_dir)
    os.makedirs(dest_app_dir, exist_ok=True)

    if os.path.isdir(src_path):
        src_app = _find_app_in_directory(src_path)
        shutil.copytree(src_app, dest_app_dir, symlinks=True, dirs_exist_ok=True)
        return

    with zipfile.ZipFile(src_path, 'r') as zf:
        app_prefix = None
        for name in zf.namelist():
            parts = name.strip('/').split('/')
            if len(parts) >= 2 and parts[0] == 'Payload' and parts[1].endswith('.app'):
                app_prefix = f'{parts[0]}/{parts[1]}/'
                break
            elif len(parts) >= 1 and parts[0].endswith('.app'):
                app_prefix = f'{parts[0]}/'
                break
        if app_prefix is None:
            raise ValueError(f'No Payload/*.app found in {src_path}')

        dest_real = os.path.realpath(dest_app_dir)
        for member in zf.infolist():
            if not member.filename.startswith(app_prefix):
                continue
            rel = member.filename[len(app_prefix):]
            if not rel:
                continue
            target_path = os.path.join(dest_app_dir, *rel.split('/'))
            target_real = os.path.realpath(os.path.abspath(target_path))
            if not (target_real == dest_real or target_real.startswith(dest_real + os.sep)):
                raise ValueError(f'Unsafe zip path traversal detected: {member.filename}')

            if member.is_dir() or member.filename.endswith('/'):
                os.makedirs(target_path, exist_ok=True)
            else:
                os.makedirs(os.path.dirname(target_path), exist_ok=True)
                with zf.open(member, 'r') as src_f, open(target_path, 'wb') as dst_f:
                    shutil.copyfileobj(src_f, dst_f)


def install_ipa(path: str, root_dir: str, trustcache_out: Optional[str] = None) -> Dict[str, Any]:
    """Install an `.ipa` or `.app` into `<root_dir>/private/var/containers/{Bundle,Data}/Application/`
    and update the InstallCoordination registry and Apple TrustCache v1."""
    app_info = inspect_ipa(path)
    bundle_id = app_info['bundle_id']
    if not bundle_id:
        raise ValueError(f'Cannot install app without CFBundleIdentifier: {path}')

    root_dir = os.path.abspath(root_dir)

    # Ensure base container directories exist
    for rel_sub, uid in (
        ('Bundle/Application', UID_INSTALLD),
        ('Data/Application', UID_MOBILE),
        ('Shared/SystemGroup', UID_ROOT),
        ('Temp', UID_ROOT),
    ):
        d = os.path.join(root_dir, 'private', 'var', 'containers', *rel_sub.split('/'))
        os.makedirs(d, exist_ok=True)
        safe_chown(d, uid, uid)
        safe_chmod(d, 0o755)

    # Reuse existing container UUIDs if upgrading an already installed app
    existing_bundle_dir, bundle_uuid = _find_existing_container(root_dir, 'Bundle/Application', bundle_id)
    existing_data_dir, data_uuid = _find_existing_container(root_dir, 'Data/Application', bundle_id)

    if not bundle_uuid:
        bundle_uuid = str(uuid.uuid4()).upper()
    if not data_uuid:
        data_uuid = str(uuid.uuid4()).upper()

    bundle_container_dir = existing_bundle_dir or os.path.join(
        root_dir, 'private', 'var', 'containers', 'Bundle', 'Application', bundle_uuid
    )
    data_container_dir = existing_data_dir or os.path.join(
        root_dir, 'private', 'var', 'containers', 'Data', 'Application', data_uuid
    )

    os.makedirs(bundle_container_dir, exist_ok=True)
    app_folder_name = app_info['app_folder'] or f"{app_info['bundle_name']}.app"
    if not app_folder_name.endswith('.app'):
        app_folder_name += '.app'
    dest_app_dir = os.path.join(bundle_container_dir, app_folder_name)

    # Extract/copy .app bundle
    _extract_to_bundle_dir(path, dest_app_dir)

    # Set ownership & executable modes on the bundle container (33:33 _installd)
    exec_paths = {b['path'] for b in app_info.get('binaries', [])}
    if app_info.get('executable'):
        exec_paths.add(app_info['executable'])
    apply_tree_permissions(dest_app_dir, UID_INSTALLD, UID_INSTALLD, exec_paths)
    safe_chown(bundle_container_dir, UID_INSTALLD, UID_INSTALLD)
    safe_chmod(bundle_container_dir, 0o755)

    # Write Bundle container metadata plist (MCMMetadataContentClass = 1)
    mcm_info = {
        'CFBundleShortVersionString': app_info['short_version'],
        'CFBundleVersion': app_info['bundle_version'],
    }
    bundle_meta = {
        'MCMMetadataIdentifier': bundle_id,
        'MCMMetadataUUID': bundle_uuid,
        'MCMMetadataContentClass': 1,
        'MCMMetadataPersona': 0,
        'MCMMetadataVersion': 1,
        'MCMMetadataInfo': mcm_info,
    }
    bundle_meta_path = os.path.join(
        bundle_container_dir, '.com.apple.mobile_container_manager.metadata.plist'
    )
    save_plist(bundle_meta_path, bundle_meta, UID_INSTALLD, UID_INSTALLD, 0o644)

    # Create Data container subdirectories & metadata plist (MCMMetadataContentClass = 2)
    os.makedirs(data_container_dir, exist_ok=True)
    safe_chown(data_container_dir, UID_MOBILE, UID_MOBILE)
    safe_chmod(data_container_dir, 0o755)
    for sub in ('Documents', 'Library', 'Library/Caches', 'Library/Preferences', 'SystemData', 'tmp'):
        sd = os.path.join(data_container_dir, *sub.split('/'))
        os.makedirs(sd, exist_ok=True)
        safe_chown(sd, UID_MOBILE, UID_MOBILE)
        safe_chmod(sd, 0o755)

    data_meta = {
        'MCMMetadataIdentifier': bundle_id,
        'MCMMetadataUUID': data_uuid,
        'MCMMetadataContentClass': 2,
        'MCMMetadataPersona': 0,
        'MCMMetadataVersion': 1,
        'MCMMetadataInfo': mcm_info,
    }
    data_meta_path = os.path.join(
        data_container_dir, '.com.apple.mobile_container_manager.metadata.plist'
    )
    save_plist(data_meta_path, data_meta, UID_MOBILE, UID_MOBILE, 0o644)

    # Update InstallCoordination registry plist
    reg_path = _registry_plist_path(root_dir)
    registry: Dict[str, Any] = {'InstalledApps': {}}
    if os.path.isfile(reg_path):
        try:
            loaded = load_plist_file(reg_path)
            if isinstance(loaded.get('InstalledApps'), dict):
                registry = loaded
        except Exception:
            pass

    rel_bundle_app = f'/private/var/containers/Bundle/Application/{bundle_uuid}/{app_folder_name}'
    rel_bundle_container = f'/private/var/containers/Bundle/Application/{bundle_uuid}'
    rel_data_container = f'/private/var/containers/Data/Application/{data_uuid}'

    entry_record = {
        'CFBundleIdentifier': bundle_id,
        'CFBundleDisplayName': app_info['display_name'],
        'CFBundleName': app_info['bundle_name'],
        'CFBundleShortVersionString': app_info['short_version'],
        'CFBundleVersion': app_info['bundle_version'],
        'CFBundleExecutable': app_info['executable'],
        'MinimumOSVersion': app_info['minimum_os_version'],
        'BundleUUID': bundle_uuid,
        'DataUUID': data_uuid,
        'BundleContainerPath': rel_bundle_container,
        'AppBundlePath': rel_bundle_app,
        'DataContainerPath': rel_data_container,
        'GraphicsPrimaryAPI': app_info['graphics']['primary_api'],
        'GraphicsAPIs': app_info['graphics']['apis'],
        'Frameworks': app_info['frameworks'],
        'CDHashes': app_info['cdhashes'],
        'ARM64Supported': app_info['arm64_supported'],
        'Encrypted': app_info['encrypted'],
        'InstalledAt': datetime.datetime.now(datetime.timezone.utc).isoformat(),
    }
    registry['InstalledApps'][bundle_id] = entry_record
    save_plist(reg_path, registry, UID_MOBILE, UID_MOBILE, 0o644)

    # Update rootfs TrustCache v1 (/private/var/db/sweet_trustcache.bin)
    default_tc_path = _default_trustcache_path(root_dir)
    tc_result = update_trustcache(default_tc_path, app_info['cdhashes'])

    extra_tc_result = None
    if trustcache_out:
        extra_tc_result = update_trustcache(trustcache_out, app_info['cdhashes'])

    return {
        'bundle_id': bundle_id,
        'display_name': app_info['display_name'],
        'short_version': app_info['short_version'],
        'bundle_version': app_info['bundle_version'],
        'bundle_uuid': bundle_uuid,
        'data_uuid': data_uuid,
        'bundle_container_dir': bundle_container_dir,
        'app_bundle_dir': dest_app_dir,
        'data_container_dir': data_container_dir,
        'registry_plist': reg_path,
        'trustcache': tc_result,
        'extra_trustcache': extra_tc_result,
        'cdhashes': app_info['cdhashes'],
        'graphics': app_info['graphics'],
        'compatible': app_info['compatible'],
        'warnings': app_info['warnings'],
    }


def list_installed_apps(root_dir: str) -> List[Dict[str, Any]]:
    """List all installed `.app` bundles in `<root_dir>/private/var/containers/Bundle/Application/`."""
    root_dir = os.path.abspath(root_dir)
    bundle_base = os.path.join(root_dir, 'private', 'var', 'containers', 'Bundle', 'Application')
    data_base = os.path.join(root_dir, 'private', 'var', 'containers', 'Data', 'Application')

    # Map bundle_id -> data_uuid from Data/Application
    data_uuid_map: Dict[str, str] = {}
    if os.path.isdir(data_base):
        for d_entry in sorted(os.listdir(data_base)):
            meta_file = os.path.join(data_base, d_entry, '.com.apple.mobile_container_manager.metadata.plist')
            if os.path.isfile(meta_file):
                try:
                    m = load_plist_file(meta_file)
                    bid = m.get('MCMMetadataIdentifier')
                    if bid:
                        data_uuid_map[str(bid)] = str(m.get('MCMMetadataUUID') or d_entry)
                except Exception:
                    pass

    apps: List[Dict[str, Any]] = []
    if not os.path.isdir(bundle_base):
        return apps

    for b_entry in sorted(os.listdir(bundle_base)):
        cdir = os.path.join(bundle_base, b_entry)
        if not os.path.isdir(cdir):
            continue

        meta_path = os.path.join(cdir, '.com.apple.mobile_container_manager.metadata.plist')
        meta = {}
        if os.path.isfile(meta_path):
            try:
                meta = load_plist_file(meta_path)
            except Exception:
                meta = {}

        app_dirs = [
            os.path.join(cdir, item)
            for item in sorted(os.listdir(cdir))
            if item.endswith('.app') and os.path.isdir(os.path.join(cdir, item))
        ]
        if not app_dirs:
            continue

        app_dir = app_dirs[0]
        try:
            inspected = inspect_ipa(app_dir)
            bundle_id = inspected['bundle_id'] or str(meta.get('MCMMetadataIdentifier') or '')
            bundle_uuid = str(meta.get('MCMMetadataUUID') or b_entry)
            apps.append({
                'bundle_id': bundle_id,
                'display_name': inspected['display_name'],
                'short_version': inspected['short_version'],
                'bundle_version': inspected['bundle_version'],
                'executable': inspected['executable'],
                'bundle_uuid': bundle_uuid,
                'data_uuid': data_uuid_map.get(bundle_id),
                'app_path': app_dir,
                'graphics_api': inspected['graphics']['primary_api'],
                'graphics_apis': inspected['graphics']['apis'],
                'frameworks': inspected['frameworks'],
                'arm64_supported': inspected['arm64_supported'],
                'encrypted': inspected['encrypted'],
                'cdhashes': inspected['cdhashes'],
                'compatible': inspected['compatible'],
            })
        except Exception as exc:
            bundle_id = str(meta.get('MCMMetadataIdentifier') or b_entry)
            apps.append({
                'bundle_id': bundle_id,
                'display_name': os.path.basename(app_dir)[:-4],
                'short_version': '?',
                'bundle_version': '?',
                'executable': '?',
                'bundle_uuid': str(meta.get('MCMMetadataUUID') or b_entry),
                'data_uuid': data_uuid_map.get(bundle_id),
                'app_path': app_dir,
                'graphics_api': 'Unknown',
                'graphics_apis': [],
                'frameworks': [],
                'arm64_supported': False,
                'encrypted': False,
                'cdhashes': [],
                'compatible': False,
                'error': str(exc),
            })

    return apps


# Alias for external callers
list_apps = list_installed_apps


def uninstall_app(bundle_id: str, root_dir: str) -> Dict[str, Any]:
    """Remove the Bundle and Data containers for `bundle_id` and update registry + TrustCache."""
    root_dir = os.path.abspath(root_dir)
    removed_bundle_dirs: List[str] = []
    removed_data_dirs: List[str] = []

    for subpath, target_list in (
        ('Bundle/Application', removed_bundle_dirs),
        ('Data/Application', removed_data_dirs),
    ):
        base = os.path.join(root_dir, 'private', 'var', 'containers', *subpath.split('/'))
        if not os.path.isdir(base):
            continue
        for entry in sorted(os.listdir(base)):
            cdir = os.path.join(base, entry)
            if not os.path.isdir(cdir):
                continue
            matched = False
            meta_path = os.path.join(cdir, '.com.apple.mobile_container_manager.metadata.plist')
            if os.path.isfile(meta_path):
                try:
                    meta = load_plist_file(meta_path)
                    if meta.get('MCMMetadataIdentifier') == bundle_id:
                        matched = True
                except Exception:
                    pass
            if not matched and subpath == 'Bundle/Application':
                for item in os.listdir(cdir):
                    info_p = os.path.join(cdir, item, 'Info.plist')
                    if item.endswith('.app') and os.path.isfile(info_p):
                        try:
                            pl = load_plist_file(info_p)
                            if pl.get('CFBundleIdentifier') == bundle_id:
                                matched = True
                                break
                        except Exception:
                            pass
            if matched:
                safe_rmtree(cdir)
                target_list.append(cdir)

    if not removed_bundle_dirs and not removed_data_dirs:
        raise FileNotFoundError(f'App "{bundle_id}" is not installed in {root_dir}')

    # Update InstallCoordination registry plist
    reg_path = _registry_plist_path(root_dir)
    if os.path.isfile(reg_path):
        try:
            reg = load_plist_file(reg_path)
            if isinstance(reg.get('InstalledApps'), dict):
                reg['InstalledApps'].pop(bundle_id, None)
                save_plist(reg_path, reg, UID_MOBILE, UID_MOBILE, 0o644)
        except Exception:
            pass

    # Rebuild TrustCache from remaining installed apps
    remaining_apps = list_installed_apps(root_dir)
    remaining_hashes: List[str] = []
    for app in remaining_apps:
        for h in app.get('cdhashes', []):
            if h not in remaining_hashes:
                remaining_hashes.append(h)

    tc_path = _default_trustcache_path(root_dir)
    if os.path.isfile(tc_path) or remaining_hashes:
        write_trustcache(tc_path, remaining_hashes)

    return {
        'bundle_id': bundle_id,
        'removed_bundle_containers': removed_bundle_dirs,
        'removed_data_containers': removed_data_dirs,
        'remaining_apps_count': len(remaining_apps),
    }


# Alias for external callers
uninstall_ipa = uninstall_app


# ---------------------------------------------------------------------------
# Demo IPA Generator (`SweetBenchmark.ipa`)
# ---------------------------------------------------------------------------

def _pack_segment_64(name: str, vmaddr: int, vmsize: int, fileoff: int, filesize: int,
                     maxprot: int, initprot: int, sections: Sequence[bytes] = ()) -> bytes:
    cmdsize = 72 + 80 * len(sections)
    hdr = struct.pack(
        '<II16sQQQQIIII',
        LC_SEGMENT_64,
        cmdsize,
        name.encode('ascii')[:16],
        vmaddr,
        vmsize,
        fileoff,
        filesize,
        maxprot,
        initprot,
        len(sections),
        0,
    )
    return hdr + b''.join(sections)


def _pack_section_64(sectname: str, segname: str, addr: int, size: int,
                     offset: int, align: int, flags: int) -> bytes:
    return struct.pack(
        '<16s16sQQIIIIIIII',
        sectname.encode('ascii')[:16],
        segname.encode('ascii')[:16],
        addr,
        size,
        offset,
        align,
        0,
        0,
        flags,
        0,
        0,
        0,
    )


def _pack_load_dylib(path: str, cmd: int = LC_LOAD_DYLIB) -> bytes:
    raw_name = path.encode('utf-8') + b'\x00'
    total_len = (24 + len(raw_name) + 7) & ~7
    padded_name = raw_name.ljust(total_len - 24, b'\x00')
    return struct.pack('<6I', cmd, total_len, 24, 2, 0x00010000, 0x00010000) + padded_name


def _build_codesig_superblob(image: bytes, code_limit: int, text_size: int,
                             ident_str: str, entitlements_bytes: bytes) -> bytes:
    """Build a SuperBlob (`0xfade0cc0`) with slot 0 SHA-256 CodeDirectory (`0xfade0c02`)
    and slot 5 XML Entitlements (`0xfade7171`)."""
    page_shift = 12
    page_size = 1 << page_shift
    ident = ident_str.encode('utf-8') + b'\x00'

    ent_blob = struct.pack('>II', CSMAGIC_EMBEDDED_ENTITLEMENTS, 8 + len(entitlements_bytes)) + entitlements_bytes
    nspecial = 5  # slots -5..-1; slot -5 is entitlements
    special_hashes = hashlib.sha256(ent_blob).digest() + (b'\x00' * (32 * 4))

    code_hashes = b''.join(
        hashlib.sha256(image[o:min(o + page_size, code_limit)]).digest()
        for o in range(0, code_limit, page_size)
    )
    nslots = len(code_hashes) // 32
    all_hashes = special_hashes + code_hashes

    hdr_len = 88
    ident_off = hdr_len
    hash_off = ident_off + len(ident) + 32 * nspecial
    cd_len = ident_off + len(ident) + len(all_hashes)

    cd = struct.pack(
        '>9I4BI',
        CSMAGIC_CODEDIRECTORY,
        cd_len,
        0x20400,
        CS_ADHOC,
        hash_off,
        ident_off,
        nspecial,
        nslots,
        code_limit,
        32,
        CS_HASHTYPE_SHA256,
        0,
        page_shift,
        0,
    )
    cd += struct.pack('>III', 0, 0, 0)
    cd += struct.pack('>QQQQ', 0, 0, text_size, 1)  # execSegFlags = CS_EXECSEG_MAIN_BINARY
    cd += ident + all_hashes

    sb_hdr_len = 12 + 8 * 2
    total_len = sb_hdr_len + len(cd) + len(ent_blob)
    sb = struct.pack('>3I', CSMAGIC_EMBEDDED_SIGNATURE, total_len, 2)
    sb += struct.pack('>IIII', CSSLOT_CODEDIRECTORY, sb_hdr_len, CSSLOT_ENTITLEMENTS, sb_hdr_len + len(cd))
    return sb + cd + ent_blob


def build_demo_macho_binary(ident_str: str = 'org.sweet.SweetBenchmark') -> bytes:
    """Construct a valid ARM64 Mach-O executable with `LC_MAIN`, `LC_LOAD_DYLIB`,
    `LC_ENCRYPTION_INFO_64 (cryptid=0)`, and `LC_CODE_SIGNATURE` (valid SHA-256 CodeDirectory)."""
    base_vm = 0x100000000
    seg_size = 0x4000  # 16 KiB page-aligned segment
    code_off = 0x800

    # Minimal position-independent ARM64 entry stub:
    #   mov w0, #0      (0x52800000)
    #   ret             (0xd65f03c0)
    arm64_code = struct.pack('<II', 0x52800000, 0xD65F03C0)

    entitlements_dict = {
        'application-identifier': f'SWEETDEMO.{ident_str}',
        'get-task-allow': True,
        'platform-application': True,
    }
    entitlements_xml = plistlib.dumps(entitlements_dict, fmt=plistlib.FMT_XML)

    # Pre-calculate exact signature size over a dummy 16K segment
    dummy_sig = _build_codesig_superblob(bytes(seg_size), seg_size, seg_size, ident_str, entitlements_xml)
    sig_size = len(dummy_sig)
    linkedit_vmsize = (sig_size + seg_size - 1) & ~(seg_size - 1)

    text_sect = _pack_section_64(
        '__text', '__TEXT', base_vm + code_off, len(arm64_code), code_off, 2, 0x80000400
    )

    dylib_paths = [
        '/usr/lib/libSystem.B.dylib',
        '/System/Library/Frameworks/Foundation.framework/Foundation',
        '/System/Library/Frameworks/UIKit.framework/UIKit',
        '/System/Library/Frameworks/QuartzCore.framework/QuartzCore',
        '/System/Library/Frameworks/CoreGraphics.framework/CoreGraphics',
        '/System/Library/Frameworks/OpenGLES.framework/OpenGLES',
        '/System/Library/Frameworks/Metal.framework/Metal',
        '/System/Library/Frameworks/GameController.framework/GameController',
    ]

    deterministic_uuid = uuid.uuid5(uuid.NAMESPACE_DNS, ident_str).bytes

    load_cmds = [
        _pack_segment_64('__PAGEZERO', 0, base_vm, 0, 0, 0, 0),
        _pack_segment_64('__TEXT', base_vm, seg_size, 0, seg_size, 5, 5, [text_sect]),
        _pack_segment_64('__LINKEDIT', base_vm + seg_size, linkedit_vmsize, seg_size, sig_size, 1, 1),
        struct.pack('<II16s', LC_UUID, 24, deterministic_uuid),
        struct.pack('<IIQQ', LC_MAIN, 24, code_off, 0),
        struct.pack('<6I', LC_ENCRYPTION_INFO_64, 24, 0, 0, 0, 0),
    ]
    for dp in dylib_paths:
        load_cmds.append(_pack_load_dylib(dp, LC_LOAD_DYLIB))
    load_cmds.append(struct.pack('<4I', LC_CODE_SIGNATURE, 16, seg_size, sig_size))

    lc_bytes = b''.join(load_cmds)
    header = struct.pack(
        '<8I',
        MH_MAGIC_64,
        CPU_TYPE_ARM64,
        0,
        MH_EXECUTE,
        len(load_cmds),
        len(lc_bytes),
        MH_NOUNDEFS | MH_DYLDLINK | MH_TWOLEVEL | MH_PIE,
        0,
    )
    assert len(header) + len(lc_bytes) <= code_off, 'Load commands exceed code_off'

    image = bytearray(seg_size)
    image[:len(header) + len(lc_bytes)] = header + lc_bytes
    image[code_off:code_off + len(arm64_code)] = arm64_code

    real_sig = _build_codesig_superblob(bytes(image), seg_size, seg_size, ident_str, entitlements_xml)
    assert len(real_sig) == sig_size
    return bytes(image) + real_sig


def create_demo_ipa(output_path: str) -> Dict[str, Any]:
    """Generate a complete, valid demo `.ipa` (`SweetBenchmark.ipa`) with an ARM64 Mach-O
    executable, SHA-256 CodeDirectory signature, XML entitlements, `Info.plist`, and `PkgInfo`."""
    output_path = os.path.abspath(output_path)
    os.makedirs(os.path.dirname(output_path), exist_ok=True)

    bundle_id = 'org.sweet.SweetBenchmark'
    app_name = 'SweetBenchmark'

    info_plist = {
        'CFBundleDevelopmentRegion': 'en',
        'CFBundleDisplayName': 'SweetBenchmark',
        'CFBundleExecutable': app_name,
        'CFBundleIdentifier': bundle_id,
        'CFBundleInfoDictionaryVersion': '6.0',
        'CFBundleName': app_name,
        'CFBundlePackageType': 'APPL',
        'CFBundleShortVersionString': '1.0.0',
        'CFBundleSupportedPlatforms': ['iPhoneOS'],
        'CFBundleVersion': '1',
        'DTPlatformName': 'iphoneos',
        'DTPlatformVersion': '16.7',
        'LSRequiresIPhoneOS': True,
        'MinimumOSVersion': '16.0',
        'UIDeviceFamily': [1, 2],
        'UIRequiredDeviceCapabilities': ['arm64', 'opengles-2', 'metal'],
        'UISupportedInterfaceOrientations': [
            'UIInterfaceOrientationPortrait',
            'UIInterfaceOrientationLandscapeLeft',
            'UIInterfaceOrientationLandscapeRight',
        ],
    }

    macho_bytes = build_demo_macho_binary(bundle_id)
    plist_bytes = plistlib.dumps(info_plist, fmt=plistlib.FMT_BINARY)

    with zipfile.ZipFile(output_path, 'w', compression=zipfile.ZIP_DEFLATED) as zf:
        zf.writestr(f'Payload/{app_name}.app/Info.plist', plist_bytes)
        zf.writestr(f'Payload/{app_name}.app/PkgInfo', b'APPL????')
        zf.writestr(f'Payload/{app_name}.app/{app_name}', macho_bytes)

    inspected = inspect_ipa(output_path)
    return {
        'output_path': output_path,
        'size_bytes': os.path.getsize(output_path),
        'bundle_id': bundle_id,
        'cdhashes': inspected['cdhashes'],
        'info': inspected,
    }


# ---------------------------------------------------------------------------
# Human-Readable Formatting & CLI Entrypoint
# ---------------------------------------------------------------------------

def _print_info_report(report: Dict[str, Any]) -> None:
    status_str = 'COMPATIBLE (Ready for ios-sweet)' if report['compatible'] else 'NEEDS ATTENTION'
    print(f"=== ios-sweet IPA / App Bundle Report ===")
    print(f"Path:                 {report['path']}")
    print(f"Bundle Folder:        {report['app_folder']}")
    print(f"Display Name:         {report['display_name']}")
    print(f"Bundle Identifier:    {report['bundle_id']}")
    print(f"Version:              {report['short_version']} ({report['bundle_version']})")
    print(f"Main Executable:      {report['executable']}")
    print(f"Minimum OS Version:   {report['minimum_os_version']}")
    print(f"Device Family:        {report['device_family']}")
    print(f"Capabilities:         {list(report['required_capabilities'].keys())}")
    print(f"ARM64 Slice Present:  {'Yes' if report['arm64_supported'] else 'NO'}")
    print(f"FairPlay Encrypted:   {'YES (cryptid=' + str(report['cryptid']) + ')' if report['encrypted'] else 'No (cryptid=0)'}")
    print(f"Graphics Profile:     {report['graphics']['primary_api']} ({', '.join(report['graphics']['apis'])})")
    print(f"Linked Frameworks:    {', '.join(report['frameworks']) if report['frameworks'] else 'None'}")
    print(f"Compatibility Status: {status_str}")

    if report['warnings']:
        print("\nWarnings:")
        for w in report['warnings']:
            print(f"  [!] {w}")

    print(f"\nMach-O Binaries ({len(report['binaries'])}):")
    for b in report['binaries']:
        role = 'MAIN' if b.get('is_main_executable') else 'LIB '
        arch = 'arm64' if b.get('is_arm64') else f"cpu={hex(b.get('cpu_type') or 0)}"
        enc = f"encrypted(cryptid={b.get('cryptid')})" if b.get('encrypted') else 'cryptid=0'
        sig_src = b.get('cdhash_source', 'unknown')
        print(f"  [{role}] {b['path']} ({arch}, {enc}, cdhash={b.get('cdhash')} [{sig_src}])")


def main(argv: Optional[List[str]] = None) -> int:
    parser = argparse.ArgumentParser(
        prog='sweet-ipa.py',
        description='Inspect, prepare, and install .ipa / .app iOS bundles into ios-sweet.',
    )
    sub = parser.add_subparsers(dest='command', required=True)

    # info
    p_info = sub.add_parser('info', help='Inspect an .ipa file or .app directory')
    p_info.add_argument('bundle_path', help='Path to .ipa file or .app directory')
    p_info.add_argument('--json', action='store_true', help='Output report as JSON')

    # install
    p_install = sub.add_parser('install', help='Install an .ipa or .app into a mounted rootfs or staging directory')
    p_install.add_argument('bundle_path', help='Path to .ipa file or .app directory')
    p_install.add_argument('--root', required=True, help='Path to mounted iOS rootfs or staging directory')
    p_install.add_argument('--trustcache', help='Optional additional output path for TrustCache v1 binary')
    p_install.add_argument('--json', action='store_true', help='Output installation result as JSON')

    # list
    p_list = sub.add_parser('list', help='List all installed .app bundles in a rootfs or staging directory')
    p_list.add_argument('--root', required=True, help='Path to mounted iOS rootfs or staging directory')
    p_list.add_argument('--json', action='store_true', help='Output installed apps list as JSON')

    # uninstall
    p_uninstall = sub.add_parser('uninstall', help='Remove bundle and data containers for a bundle identifier')
    p_uninstall.add_argument('bundle_id', help='CFBundleIdentifier of the app to remove')
    p_uninstall.add_argument('--root', required=True, help='Path to mounted iOS rootfs or staging directory')
    p_uninstall.add_argument('--json', action='store_true', help='Output uninstall result as JSON')

    # create-demo-ipa
    p_demo = sub.add_parser('create-demo-ipa', help='Generate a valid self-contained SweetBenchmark.ipa demo archive')
    p_demo.add_argument('output_ipa', help='Output .ipa file path (e.g. SweetBenchmark.ipa)')
    p_demo.add_argument('--json', action='store_true', help='Output demo creation result as JSON')

    args = parser.parse_args(argv)

    try:
        if args.command == 'info':
            report = inspect_ipa(args.bundle_path)
            if args.json:
                print(json.dumps(report, indent=2))
            else:
                _print_info_report(report)
            return 0

        elif args.command == 'install':
            res = install_ipa(args.bundle_path, args.root, trustcache_out=args.trustcache)
            if args.json:
                print(json.dumps(res, indent=2))
            else:
                print(f"Installed {res['display_name']} ({res['bundle_id']}) v{res['short_version']}")
                print(f"  Bundle UUID:      {res['bundle_uuid']}")
                print(f"  Data UUID:        {res['data_uuid']}")
                print(f"  App Bundle Path:  {res['app_bundle_dir']}")
                print(f"  Data Container:   {res['data_container_dir']}")
                print(f"  Registry Plist:   {res['registry_plist']}")
                print(f"  TrustCache:       {res['trustcache']['path']} ({res['trustcache']['num_entries']} entries)")
                if res.get('extra_trustcache'):
                    print(f"  Extra TrustCache: {res['extra_trustcache']['path']}")
                for w in res.get('warnings', []):
                    print(f"  [!] Warning: {w}")
            return 0

        elif args.command == 'list':
            apps = list_installed_apps(args.root)
            if args.json:
                print(json.dumps(apps, indent=2))
            else:
                if not apps:
                    print(f"No installed apps found in {os.path.abspath(args.root)}")
                else:
                    print(f"Installed Apps in {os.path.abspath(args.root)} ({len(apps)}):")
                    for a in apps:
                        print(
                            f"  - {a['bundle_id']} ({a['display_name']} v{a['short_version']}) | "
                            f"UUID={a['bundle_uuid']} | DataUUID={a.get('data_uuid') or 'None'} | "
                            f"API={a['graphics_api']}"
                        )
            return 0

        elif args.command == 'uninstall':
            res = uninstall_app(args.bundle_id, args.root)
            if args.json:
                print(json.dumps(res, indent=2))
            else:
                print(f"Uninstalled {res['bundle_id']}:")
                for p in res['removed_bundle_containers']:
                    print(f"  Removed Bundle Container: {p}")
                for p in res['removed_data_containers']:
                    print(f"  Removed Data Container:   {p}")
            return 0

        elif args.command == 'create-demo-ipa':
            res = create_demo_ipa(args.output_ipa)
            if args.json:
                print(json.dumps(res, indent=2))
            else:
                print(f"Created demo IPA: {res['output_path']} ({res['size_bytes']} bytes)")
                print(f"  Bundle ID: {res['bundle_id']}")
                print(f"  CDHashes:  {', '.join(res['cdhashes'])}")
            return 0

    except Exception as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1

    return 0


if __name__ == '__main__':
    sys.exit(main())
