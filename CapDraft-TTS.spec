# -*- mode: python ; coding: utf-8 -*-

from PyInstaller.utils.hooks import collect_data_files

datas = []
datas += collect_data_files("qfluentwidgets")
# CapCut TTS HTTP client + device defaults (resolved via APP_ROOT/external/...)
datas += [
    ("external/capcut-tts-api/capcut_common_task_client.py", "external/capcut-tts-api"),
    ("external/capcut-tts-api/device.json", "external/capcut-tts-api"),
]

a = Analysis(
    ["main.py"],
    pathex=[],
    binaries=[],
    datas=datas,
    hiddenimports=[],
    hookspath=[],
    hooksconfig={},
    runtime_hooks=[],
    excludes=[
        "torch", "transformers", "scipy", "sklearn", "pandas", "numpy",
        "matplotlib", "notebook", "jupyter", "IPython", "pyarrow",
    ],
    noarchive=False,
    optimize=1,
)
pyz = PYZ(a.pure)

exe = EXE(
    pyz,
    a.scripts,
    [],
    exclude_binaries=True,
    name="CapDraft-TTS",
    debug=False,
    bootloader_ignore_signals=False,
    strip=False,
    upx=True,
    console=False,
    disable_windowed_traceback=False,
    argv_emulation=False,
    target_arch=None,
    codesign_identity=None,
    entitlements_file=None,
)

coll = COLLECT(
    exe,
    a.binaries,
    a.datas,
    strip=False,
    upx=True,
    upx_exclude=[],
    name="CapDraft-TTS",
)
