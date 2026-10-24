"""Verify Windows product metadata and embedded icon bytes in built binaries."""
import ctypes
from ctypes import wintypes
import struct


def verify_branding(repo, output, expected_version=None, products=None):
    kernel = ctypes.WinDLL('kernel32', use_last_error=True)
    version = ctypes.WinDLL('version', use_last_error=True)
    kernel.LoadLibraryExW.argtypes = [wintypes.LPCWSTR, wintypes.HANDLE, wintypes.DWORD]
    kernel.LoadLibraryExW.restype = wintypes.HMODULE
    kernel.FindResourceW.argtypes = [wintypes.HMODULE, ctypes.c_void_p, ctypes.c_void_p]
    kernel.FindResourceW.restype = ctypes.c_void_p
    kernel.LoadResource.argtypes = [wintypes.HMODULE, ctypes.c_void_p]
    kernel.LoadResource.restype = ctypes.c_void_p
    kernel.LockResource.argtypes = [ctypes.c_void_p]
    kernel.LockResource.restype = ctypes.c_void_p
    kernel.SizeofResource.argtypes = [wintypes.HMODULE, ctypes.c_void_p]
    kernel.SizeofResource.restype = wintypes.DWORD
    kernel.FreeLibrary.argtypes = [wintypes.HMODULE]
    callback_type = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HMODULE,
                                      ctypes.c_void_p, ctypes.c_void_p, wintypes.LPARAM)
    kernel.EnumResourceNamesW.argtypes = [wintypes.HMODULE, ctypes.c_void_p,
                                        callback_type, wintypes.LPARAM]
    version.GetFileVersionInfoSizeW.argtypes = [wintypes.LPCWSTR, ctypes.c_void_p]
    version.GetFileVersionInfoSizeW.restype = wintypes.DWORD
    version.GetFileVersionInfoW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD,
                                          wintypes.DWORD, ctypes.c_void_p]
    version.VerQueryValueW.argtypes = [ctypes.c_void_p, wintypes.LPCWSTR,
                                      ctypes.POINTER(ctypes.c_void_p), ctypes.POINTER(wintypes.UINT)]

    icon = (repo / 'windows/assets/mori.ico').read_bytes()
    expected_images = set()
    for index in range(struct.unpack_from('<H', icon, 4)[0]):
        size, offset = struct.unpack_from('<II', icon, 6 + 16 * index + 8)
        expected_images.add(icon[offset:offset + size])

    if products is None:
        products = [('chrome.exe', 'Mori'), ('setup.exe', 'Mori Installer'),
                    ('mini_installer.exe', 'Mori Installer')]
    for filename, product_name in products:
        path = str(output / filename)
        size = version.GetFileVersionInfoSizeW(path, None)
        if not size:
            raise ctypes.WinError(ctypes.get_last_error())
        metadata = ctypes.create_string_buffer(size)
        if not version.GetFileVersionInfoW(path, 0, size, metadata):
            raise ctypes.WinError(ctypes.get_last_error())

        def query(key):
            value, length = ctypes.c_void_p(), wintypes.UINT()
            if not version.VerQueryValueW(metadata, key, ctypes.byref(value), ctypes.byref(length)):
                raise RuntimeError(f'{filename}: missing version field {key}')
            return value, length.value

        translations, _ = query(r'\VarFileInfo\Translation')
        language, code_page = struct.unpack('<HH', ctypes.string_at(translations, 4))
        prefix = f'\\StringFileInfo\\{language:04x}{code_page:04x}\\'
        fields = [('ProductName', product_name), ('CompanyName', 'Mori Browser')]
        if expected_version:
            fields.append(('ProductVersion', expected_version))
        for field, expected in fields:
            # Inno Setup pads string resources with spaces.
            actual = ctypes.wstring_at(query(prefix + field)[0]).rstrip(' ')
            if actual != expected:
                raise RuntimeError(f'{filename}: {field} is {actual!r}, expected {expected!r}')

        handle = kernel.LoadLibraryExW(path, None, 2)  # Read resources; never execute the binary.
        if not handle:
            raise ctypes.WinError(ctypes.get_last_error())
        try:
            names = []

            @callback_type
            def collect(_handle, _type, name, _parameter):
                names.append(name if name <= 65535 else ctypes.wstring_at(name))
                return True

            if not kernel.EnumResourceNamesW(handle, ctypes.c_void_p(14), collect, 0):
                raise ctypes.WinError(ctypes.get_last_error())

            def resource(kind, identifier):
                pointer = (ctypes.c_void_p(identifier) if isinstance(identifier, int) else
                           ctypes.cast(ctypes.c_wchar_p(identifier), ctypes.c_void_p))
                found = kernel.FindResourceW(handle, pointer, ctypes.c_void_p(kind))
                if not found:
                    raise ctypes.WinError(ctypes.get_last_error())
                return ctypes.string_at(kernel.LockResource(kernel.LoadResource(handle, found)),
                                        kernel.SizeofResource(handle, found))

            primary = 'IDR_MAINFRAME' if 'IDR_MAINFRAME' in names else names[0]
            group = resource(14, primary)
            images = {resource(3, struct.unpack_from('<H', group, 6 + 14 * index + 12)[0])
                      for index in range(struct.unpack_from('<H', group, 4)[0])}
            if images != expected_images:
                raise RuntimeError(f'{filename}: primary Windows icon does not match Mori artwork')
        finally:
            kernel.FreeLibrary(handle)
        print(f'Verified {filename}: {product_name}, original Mori icon at {len(images)} sizes')
