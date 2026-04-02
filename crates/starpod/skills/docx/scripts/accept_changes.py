"""Accept all tracked changes in a DOCX file using LibreOffice.

Usage:
    python accept_changes.py input.docx output.docx
"""

import argparse
import os
import shutil
import subprocess
import sys
from pathlib import Path

try:
    from office.soffice import get_soffice_env
except ImportError:
    def get_soffice_env():
        env = os.environ.copy()
        env["SAL_USE_VCLPLUGIN"] = "svp"
        return env

PROFILE_DIR = "/tmp/libreoffice_docx_profile"
MACRO_DIR = f"{PROFILE_DIR}/user/basic/Standard"

ACCEPT_MACRO = """<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE script:module PUBLIC "-//OpenOffice.org//DTD OfficeDocument 1.0//EN" "module.dtd">
<script:module xmlns:script="http://openoffice.org/2000/script" script:name="Module1" script:language="StarBasic">
    Sub AcceptAllTrackedChanges()
        Dim document As Object
        Dim dispatcher As Object
        document = ThisComponent.CurrentController.Frame
        dispatcher = createUnoService("com.sun.star.frame.DispatchHelper")
        dispatcher.executeDispatch(document, ".uno:AcceptAllTrackedChanges", "", 0, Array())
        ThisComponent.store()
        ThisComponent.close(True)
    End Sub
</script:module>"""


def setup_macro() -> bool:
    macro_dir = Path(MACRO_DIR)
    macro_file = macro_dir / "Module1.xba"

    if macro_file.exists() and "AcceptAllTrackedChanges" in macro_file.read_text():
        return True

    if not macro_dir.exists():
        subprocess.run(
            ["soffice", "--headless", f"-env:UserInstallation=file://{PROFILE_DIR}", "--terminate_after_init"],
            capture_output=True, timeout=10, check=False, env=get_soffice_env(),
        )
        macro_dir.mkdir(parents=True, exist_ok=True)

    try:
        macro_file.write_text(ACCEPT_MACRO)
        return True
    except Exception:
        return False


def accept_changes(input_file: str, output_file: str) -> str:
    input_path = Path(input_file)
    output_path = Path(output_file)

    if not input_path.exists():
        return f"Error: Input file not found: {input_file}"

    if input_path.suffix.lower() != ".docx":
        return f"Error: Not a DOCX file: {input_file}"

    output_path.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(input_path, output_path)

    if not setup_macro():
        return "Error: Failed to setup LibreOffice macro"

    cmd = [
        "soffice", "--headless",
        f"-env:UserInstallation=file://{PROFILE_DIR}",
        "--norestore",
        "vnd.sun.star.script:Standard.Module1.AcceptAllTrackedChanges?language=Basic&location=application",
        str(output_path.absolute()),
    ]

    try:
        subprocess.run(cmd, capture_output=True, text=True, timeout=30, check=False, env=get_soffice_env())
    except subprocess.TimeoutExpired:
        pass  # Timeout is expected — macro exits after completion

    return f"Accepted all tracked changes: {input_file} -> {output_file}"


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Accept all tracked changes in a DOCX")
    parser.add_argument("input_file", help="Input DOCX with tracked changes")
    parser.add_argument("output_file", help="Output DOCX (clean)")
    args = parser.parse_args()

    msg = accept_changes(args.input_file, args.output_file)
    print(msg)
    if "Error" in msg:
        sys.exit(1)
