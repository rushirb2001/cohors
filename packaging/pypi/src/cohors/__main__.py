"""Launcher: hand off to the native `cohors` binary bundled in this package.

`pip install cohors` fetches the platform wheel for this machine, which carries
the matching prebuilt binary. The `cohors` console script points here; we just
exec the binary, forwarding every argument and the exit code.
"""

import os
import sys


def _binary_path():
    here = os.path.dirname(os.path.abspath(__file__))
    name = "cohors.exe" if os.name == "nt" else "cohors"
    return os.path.join(here, name)


def main():
    exe = _binary_path()
    if not os.path.exists(exe):
        sys.exit(
            "cohors: bundled binary not found (%s). This wheel may not match "
            "your platform — install from https://github.com/rushirb2001/cohors "
            "instead." % exe
        )
    # Wheels can lose the exec bit through packaging; restore it defensively.
    try:
        os.chmod(exe, 0o755)
    except OSError:
        pass

    argv = [exe, *sys.argv[1:]]
    # Tell cohors how it was installed, so it can keep itself up to date:
    # `uvx` runs it from uv's own folders (a new release arrives on the next
    # `uvx cohors@latest`), anything else is pip, through this interpreter.
    env = dict(os.environ)
    from_uv = os.sep + "uv" + os.sep in os.path.abspath(sys.prefix)
    env["COHORS_INSTALLER"] = "uvx" if from_uv else "pip"
    env["COHORS_PYTHON"] = sys.executable
    if os.name == "nt":
        import subprocess

        raise SystemExit(subprocess.run(argv, env=env).returncode)
    # POSIX: replace this process so signals/TTY behave exactly as the binary's.
    os.execve(exe, argv, env)


if __name__ == "__main__":
    main()
