"""Check that the built exes carry the app icon as an icon group resource.

    py -3 tools\\check_exe_icon.py [exe ...]

Default: target\\release\\CANoxide.exe and target\\release\\canoxide-cli.exe.
The exe is loaded as a data file only (no code runs, no window opens). Exit
code 0 when every exe has an RT_GROUP_ICON resource whose entries match the
sizes of assets\\icon\\canoxide.ico; 1 otherwise. Standard library only.
"""

import ctypes
import os
import struct
import sys
from ctypes import wintypes

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ICO = os.path.join(ROOT, "assets", "icon", "canoxide.ico")
DEFAULT = [os.path.join(ROOT, "target", "release", n) for n in ("CANoxide.exe", "canoxide-cli.exe")]

RT_GROUP_ICON = 14
LOAD_LIBRARY_AS_DATAFILE = 0x02
LOAD_LIBRARY_AS_IMAGE_RESOURCE = 0x20

k32 = ctypes.WinDLL("kernel32", use_last_error=True)
k32.LoadLibraryExW.restype = wintypes.HMODULE
k32.LoadLibraryExW.argtypes = [wintypes.LPCWSTR, wintypes.HANDLE, wintypes.DWORD]
k32.FreeLibrary.argtypes = [wintypes.HMODULE]
k32.FindResourceW.restype = wintypes.HANDLE
k32.FindResourceW.argtypes = [wintypes.HMODULE, wintypes.LPVOID, wintypes.LPVOID]
k32.LoadResource.restype = wintypes.HANDLE
k32.LoadResource.argtypes = [wintypes.HMODULE, wintypes.HANDLE]
k32.LockResource.restype = ctypes.c_void_p
k32.LockResource.argtypes = [wintypes.HANDLE]
k32.SizeofResource.restype = wintypes.DWORD
k32.SizeofResource.argtypes = [wintypes.HMODULE, wintypes.HANDLE]
ENUMPROC = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HMODULE, wintypes.LPVOID, wintypes.LPVOID, wintypes.LPARAM)
k32.EnumResourceNamesW.argtypes = [wintypes.HMODULE, wintypes.LPVOID, ENUMPROC, wintypes.LPARAM]


def ico_sizes(path):
    """(width, height) of each entry of an .ico file (0 means 256)."""
    with open(path, "rb") as f:
        data = f.read()
    _, kind, count = struct.unpack_from("<HHH", data, 0)
    if kind != 1:
        raise ValueError(f"{path}: not an icon file")
    return sorted((data[6 + i * 16] or 256, data[7 + i * 16] or 256) for i in range(count))


def group_icons(exe):
    """Entry sizes of each RT_GROUP_ICON resource of the exe."""
    h = k32.LoadLibraryExW(exe, None, LOAD_LIBRARY_AS_DATAFILE | LOAD_LIBRARY_AS_IMAGE_RESOURCE)
    if not h:
        raise OSError(f"{exe}: cannot be loaded as a data file (error {ctypes.get_last_error()})")
    names = []
    try:
        def cb(_h, _type, name, _l):
            names.append(name)
            return True

        k32.EnumResourceNamesW(h, ctypes.c_void_p(RT_GROUP_ICON), ENUMPROC(cb), 0)
        groups = []
        for name in names:
            r = k32.FindResourceW(h, ctypes.c_void_p(name) if isinstance(name, int) else name, ctypes.c_void_p(RT_GROUP_ICON))
            size = k32.SizeofResource(h, r)
            p = k32.LockResource(k32.LoadResource(h, r))
            raw = ctypes.string_at(p, size)
            _, kind, count = struct.unpack_from("<HHH", raw, 0)
            # GRPICONDIRENTRY: 14 bytes each.
            groups.append(sorted((raw[6 + i * 14] or 256, raw[7 + i * 14] or 256) for i in range(count)) if kind == 1 else [])
        return groups
    finally:
        k32.FreeLibrary(h)


def main(argv):
    exes = argv or DEFAULT
    want = ico_sizes(ICO)
    ok = True
    for exe in exes:
        if not os.path.isfile(exe):
            print(f"FAIL {exe}: not found (build with cargo build --release)")
            ok = False
            continue
        groups = group_icons(exe)
        if not groups:
            print(f"FAIL {exe}: no icon group resource")
            ok = False
        elif want not in groups:
            print(f"FAIL {exe}: icon group sizes {groups[0]} differ from canoxide.ico {want}")
            ok = False
        else:
            print(f"OK   {exe}: icon group with {len(want)} sizes ({', '.join(str(w) for w, _ in want)} px)")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
