#!/usr/bin/env python3
"""Resolve Pi's model without reading or printing its authentication file."""
import json
import os
import sys
from pathlib import Path


def main():
    provider, model = os.getenv("PI_PROVIDER"), os.getenv("PI_MODEL")
    if bool(provider) != bool(model):
        raise SystemExit("Set both PI_PROVIDER and PI_MODEL, or neither")
    if not provider:
        try:
            settings = json.loads(Path(sys.argv[1]).read_text())
            provider, model = settings["defaultProvider"], settings["defaultModel"]
        except (OSError, ValueError, KeyError) as error:
            raise SystemExit(f"Cannot resolve Pi's default model: {error}")
    if not all(isinstance(value, str) and value and not any(c.isspace() for c in value)
               for value in (provider, model)):
        raise SystemExit("Pi provider and model must be nonempty names without whitespace")
    print(provider, model)


if __name__ == "__main__":
    main()
