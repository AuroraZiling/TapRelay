"""Resolve valid Developer ID identities from `security find-identity -v` output."""
import argparse
import re
import sys


def resolve(output, requested, installer=False):
    kind = "Developer ID Installer" if installer else "Developer ID Application"
    variable = "TAPRELAY_INSTALL_IDENTITY" if installer else "TAPRELAY_SIGN_IDENTITY"
    identities = {
        fingerprint.upper(): name
        for fingerprint, name in re.findall(
            r'^\s*\d+\)\s+([0-9A-Fa-f]{40})\s+"(.*)"\s*$', output, re.MULTILINE
        )
        if name.startswith(f"{kind}:")
    }
    if not identities:
        raise ValueError(
            f"No valid {kind} signing identity in the keychain. "
            "Check that the P12 contains both the certificate and its private key, "
            "and that the certificate is valid and trusted."
        )
    if requested:
        identities = {
            fingerprint: name
            for fingerprint, name in identities.items()
            if requested.upper() == fingerprint or requested == name
        }
        if not identities:
            raise ValueError(
                f"{variable} does not match a valid {kind} "
                "identity in the keychain. Use its exact certificate name or SHA-1 fingerprint."
            )
    if len(identities) != 1:
        raise ValueError(
            f"Multiple {kind} identities found. Export only the intended "
            f"identity for CI, or set {variable} explicitly for local packaging."
        )
    fingerprint, name = next(iter(identities.items()))
    return name if installer else fingerprint


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("requested", nargs="?", default="")
    parser.add_argument("--installer", action="store_true")
    args = parser.parse_args()
    try:
        print(resolve(sys.stdin.read(), args.requested, args.installer))
    except ValueError as error:
        print(f"Code signing: {error}", file=sys.stderr)
        sys.exit(1)
