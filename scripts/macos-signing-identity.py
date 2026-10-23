"""Resolve valid Developer ID identities from `security find-identity -v` output."""
import re
import sys


def resolve(output, requested):
    identities = {
        fingerprint.upper(): name
        for fingerprint, name in re.findall(
            r'^\s*\d+\)\s+([0-9A-Fa-f]{40})\s+"(.*)"\s*$', output, re.MULTILINE
        )
        if name.startswith("Developer ID Application:")
    }
    if not identities:
        raise ValueError(
            "No valid Developer ID Application signing identity in the keychain. "
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
                "TAPRELAY_SIGN_IDENTITY does not match a valid Developer ID Application "
                "identity in the keychain. Use its exact certificate name or SHA-1 fingerprint."
            )
    if len(identities) != 1:
        raise ValueError(
            "Multiple Developer ID Application identities found. Export only the intended "
            "identity for CI, or set TAPRELAY_SIGN_IDENTITY explicitly for local packaging."
        )
    return next(iter(identities))


if __name__ == "__main__":
    try:
        print(resolve(sys.stdin.read(), sys.argv[1] if len(sys.argv) > 1 else ""))
    except ValueError as error:
        print(f"Code signing: {error}", file=sys.stderr)
        sys.exit(1)
