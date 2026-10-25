"""Over-the-air updates for the dimmer.

    python ota.py keygen     one-off: makes the signing key, prints OTA_PUBLIC_KEY for .env
    python ota.py usb        first install over USB: bootloader + firmware in one UF2,
                             copied onto a Pico in BOOTSEL mode
    python ota.py push       builds and signs the firmware, puts it on the home
                             server and tells the dimmer to update

The dimmer only takes an image signed with the key whose public half was in
.env when its running firmware was built. The private key stays outside the
repo, in ~/.config/pico_pi_dimmer/ota-key.pem (or $OTA_KEY).

usb and push build with cargo, so run them where the .env applies (direnv, or
`set -a; . ./.env`). push reads MQTT_BROKER, MQTT_USER, MQTT_PASSWORD and
NODE_ID from it, and where to put the image:

    OTA_SSH   ssh target that may write the image directory, e.g. admin@family-server
    OTA_DIR   that directory, default /var/lib/smarthome-firmware
    OTA_URL   where the dimmers fetch it from, default http://<MQTT_BROKER>/fw

It needs openssl, ssh and mosquitto_pub/_sub, all in the flake's shell.
"""

import hashlib
import os
import struct
import subprocess
import sys
import tempfile
import time
from pathlib import Path

HERE = Path(__file__).parent
ELF = HERE / "target/thumbv6m-none-eabi/release/pico-dimmer"
BOOTLOADER = HERE / "bootloader/target/thumbv6m-none-eabi/release/pico-dimmer-bootloader"
KEY = Path(os.environ.get("OTA_KEY", Path.home() / ".config/pico_pi_dimmer/ota-key.pem"))
# Partitions in memory.x.
STATE, ACTIVE, ACTIVE_SIZE = 0x10006000, 0x10007000, 960 * 1024
RP2040_FAMILY = 0xE48BFF56


def keygen():
    if KEY.exists():
        sys.exit(f"{KEY} exists already; delete it first to make a new key")
    KEY.parent.mkdir(parents=True, exist_ok=True)
    old = os.umask(0o077)
    try:
        subprocess.run(["openssl", "genpkey", "-algorithm", "ed25519", "-out", str(KEY)], check=True)
    finally:
        os.umask(old)
    print(f"private key: {KEY} -- keep it, and a backup, out of git")
    print(f"put this in .env of v3 and v4, then flash once over USB:\nOTA_PUBLIC_KEY={public_key().hex()}")


def public_key():
    der = subprocess.run(
        ["openssl", "pkey", "-in", str(KEY), "-pubout", "-outform", "DER"], check=True, capture_output=True
    ).stdout
    return der[-32:]


def build(path):
    subprocess.run(["cargo", "build", "--release"], cwd=path, check=True)


def segments(elf):
    """(flash address, bytes) of the ELF's load segments."""
    d = elf.read_bytes()
    if d[:4] != b"\x7fELF":
        sys.exit(f"{elf} is not an ELF file")
    phoff, = struct.unpack_from("<I", d, 28)
    phentsize, phnum = struct.unpack_from("<HH", d, 42)
    for i in range(phnum):
        kind, off, _, paddr, filesz, *_ = struct.unpack_from("<8I", d, phoff + i * phentsize)
        if kind == 1 and filesz:
            yield paddr, d[off : off + filesz]


def image(elf):
    """The flash bytes of the active partition."""
    out = bytearray()
    for paddr, data in segments(elf):
        filesz = len(data)
        if not ACTIVE <= paddr < ACTIVE + ACTIVE_SIZE:
            sys.exit(f"segment at {paddr:#x} lies outside the active partition: built for the old layout?")
        start = paddr - ACTIVE
        out.extend(b"\xff" * max(0, start + filesz - len(out)))
        out[start : start + filesz] = data
    if len(out) > ACTIVE_SIZE:
        sys.exit(f"image is {len(out)} bytes, the partition {ACTIVE_SIZE}")
    return bytes(out)


def sign(data):
    # embassy-boot checks ed25519 over the image's SHA-512.
    with tempfile.NamedTemporaryFile() as digest:
        digest.write(hashlib.sha512(data).digest())
        digest.flush()
        return subprocess.run(
            ["openssl", "pkeyutl", "-sign", "-rawin", "-inkey", str(KEY), "-in", digest.name],
            check=True,
            capture_output=True,
        ).stdout


def usb():
    build(HERE / "bootloader")
    build(HERE)
    # Bootloader, a blank state sector (whatever was there before must not
    # read as a pending update), then the firmware.
    parts = [*segments(BOOTLOADER), (STATE, b"\xff" * 4096), (ACTIVE, image(ELF))]
    pages = {}
    for addr, data in parts:
        for i, byte in enumerate(data):
            page = pages.setdefault((addr + i) & ~255, bytearray(b"\xff" * 256))
            page[(addr + i) & 255] = byte
    # One unbroken run of pages: the RP2040 bootrom did not write the
    # firmware behind the gap after the bootloader.
    for addr in range(min(pages), max(pages), 256):
        pages.setdefault(addr, bytearray(b"\xff" * 256))
    blocks = b"".join(
        struct.pack("<8I", 0x0A324655, 0x9E5D5157, 0x2000, addr, 256, n, len(pages), RP2040_FAMILY)
        + pages[addr]
        + bytes(476 - 256)
        + struct.pack("<I", 0x0AB16F30)
        for n, addr in enumerate(sorted(pages))
    )
    uf2 = ELF.parent / "dimmer.uf2"
    uf2.write_bytes(blocks)
    drives = [*Path("/run/media").glob("*/RPI-RP2"), *Path("/media").glob("*/RPI-RP2")]
    if not drives:
        sys.exit(f"wrote {uf2}; no Pico in BOOTSEL mode mounted, copy it onto one yourself")
    time.sleep(2)  # freshly mounted drives refused the write
    (drives[0] / "dimmer.uf2").write_bytes(blocks)
    os.sync()
    print(f"flashed {drives[0]}: bootloader and firmware")


def push(elf=None):
    if elf is None:
        build(HERE)
        elf = ELF
    env = os.environ
    broker, node = env["MQTT_BROKER"], env.get("NODE_ID") or "dimmer"
    auth = ["-h", broker]
    if env.get("MQTT_USER"):
        auth += ["-u", env["MQTT_USER"], "-P", env.get("MQTT_PASSWORD", "")]
    base = f"smarthome/{node}"

    data = image(elf)
    signature = sign(data)

    # Uploaded under a hidden name and moved in, so nginx never hands out a
    # half-written file.
    target, folder = env["OTA_SSH"], env.get("OTA_DIR", "/var/lib/smarthome-firmware")
    version = subprocess.run(
        ["cargo", "pkgid"], cwd=HERE, check=True, capture_output=True, text=True
    ).stdout.strip().rsplit("@", 1)[-1].rsplit("#", 1)[-1]
    name = f"{node}-{version}-{hashlib.sha256(data).hexdigest()[:7]}.bin"
    with tempfile.NamedTemporaryFile(suffix=".bin") as tmp:
        tmp.write(data)
        tmp.flush()
        subprocess.run(["scp", "-q", tmp.name, f"{target}:{folder}/.{name}"], check=True)
    subprocess.run(["ssh", target, f"mv {folder}/.{name} {folder}/{name}"], check=True)
    url = f"{env.get('OTA_URL', f'http://{broker}/fw')}/{name}"

    def sub(topic, seconds):
        return subprocess.Popen(
            ["mosquitto_sub", *auth, "-t", topic, "-C", "1", "-W", str(seconds)],
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
        )

    result = sub(f"{base}/ota/result", 120)
    print(f"{node}: {len(data)} bytes from {url}")
    subprocess.run(
        ["mosquitto_pub", *auth, "-t", f"{base}/ota", "-m", f"{url} {len(data)} {signature.hex()}"], check=True
    )
    answer = result.communicate()[0].strip()
    if not answer:
        sys.exit("no answer from the dimmer within 2 minutes: offline, or its firmware has no OTA yet")
    print(f"dimmer: {answer}")
    if not answer.startswith("verified"):
        sys.exit(1)

    # Retained status: offline (last will) while it swaps, online once it runs again.
    for want in ("offline", "online"):
        watch = subprocess.Popen(
            ["mosquitto_sub", *auth, "-t", f"{base}/status", "-W", "120"],
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
        )
        line = ""
        for line in watch.stdout:
            if line.strip() == want:
                break
        watch.kill()
        if line.strip() != want:
            sys.exit(f"dimmer did not come back {want}")
    print("dimmer is back online with the new firmware")


if __name__ == "__main__":
    match sys.argv[1:]:
        case ["keygen"]:
            keygen()
        case ["usb"]:
            usb()
        case ["push"]:
            push()
        case ["push", elf]:
            push(Path(elf))
        case _:
            sys.exit(__doc__)
