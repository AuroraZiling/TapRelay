import subprocess
import sys
import unittest
from pathlib import Path


SCRIPT = Path(__file__).resolve().parents[1] / "macos-signing-identity.py"
APP = "Developer ID Application: TapRelay (TESTTEAM01)"
INSTALLER = "Developer ID Installer: TapRelay (TESTTEAM01)"
IDENTITIES = f'1) {"A" * 40} "{APP}"\n2) {"B" * 40} "{INSTALLER}"\n'


def select(*args, output=IDENTITIES):
    return subprocess.run(
        [sys.executable, str(SCRIPT), *args], input=output, text=True, capture_output=True
    )


class SigningIdentityTests(unittest.TestCase):
    def test_application_keeps_fingerprint_for_codesign(self):
        self.assertEqual(select(APP).stdout.strip(), "A" * 40)

    def test_installer_selects_common_name_for_productbuild(self):
        self.assertEqual(select("--installer").stdout.strip(), INSTALLER)
        self.assertEqual(select("--installer", "b" * 40).stdout.strip(), INSTALLER)

    def test_installer_rejects_an_application_identity(self):
        result = select("--installer", output=IDENTITIES.splitlines()[0])
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("No valid Developer ID Installer", result.stderr)

    def test_ambiguous_installer_requires_explicit_selection(self):
        second = f'3) {"C" * 40} "Developer ID Installer: Other (SECONDTEAM)"\n'
        self.assertNotEqual(select("--installer", output=IDENTITIES + second).returncode, 0)
        self.assertEqual(select("--installer", INSTALLER, output=IDENTITIES + second).stdout.strip(), INSTALLER)
