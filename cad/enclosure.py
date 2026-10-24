"""Enclosure v1 for the v3 dimmer: Pico + Pimoroni Wireless Pack, MOSFET
module and 24 V -> 5 V buck converter in one 60 x 25 x 45 mm box. Both cables
(24 V in, LED strip out) leave through the same end face.

Two parts, both print without support:

    enclosure_v1_body   open box, floor on the bed
    enclosure_v1_lid    plate on the bed, lip up; press-fits into the body

Layout inside, the box standing on its floor:

    The Wireless Pack is 53 x 25 mm -- wider than the 21.8 mm inside -- so the
    Pico + pack stack stands on its long edge on the floor, the pack's 25 mm
    upright. Above it, the MOSFET module and the buck converter sit on the
    underside of the lid (double-sided foam tape), right at the height of the
    cable holes. Cinch a cable tie round each cable inside the wall as strain
    relief.

Run in the CAD shell:  nix develop .#cad  &&  python cad/enclosure.py
"""

from pathlib import Path

import cadquery as cq
from cadquery import exporters

# Outside, as specified. The lid's plate is part of the height.
L, W, H = 60.0, 25.0, 45.0
WALL = 1.6  # walls and floor
LID = 1.6  # lid plate
R = 2.0  # outer vertical corner radius

LIP = 3.0  # how far the lid's lip reaches into the body
LIP_T = 1.2  # lip wall thickness
FIT = 0.15  # lip-to-wall clearance per side: lid too loose -> smaller

CABLE_D = 5.5  # cable hole diameter: the cable's outside + ~0.5
CABLE_GAP = 11.0  # centre-to-centre, side by side across the end face

# What has to fit (mm). STACK is the Pico + pack sandwich, plugged together,
# from the pack's top components to the Pico's -- measure yours.
PACK_L, PACK_W, STACK = 53.0, 25.0, 18.0
MOSFET = (34.0, 17.0, 12.0)  # dual-MOSFET trigger module incl. screw terminals
BUCK = (17.2, 11.2, 4.0)  # Mini-360; an MP1584 (22 x 17) fits as well

IN_L, IN_W = L - 2 * WALL, W - 2 * WALL
BODY_H = H - LID
IN_H = BODY_H - WALL  # floor to the underside of the lid

# Pack on edge on the floor, MOSFET hanging from the lid above it.
assert IN_L >= PACK_L + 1 and IN_W >= STACK + 1, "board stack does not fit"
assert IN_H >= PACK_W + MOSFET[2] + 2, "no room above the stack for the MOSFET module"
LIP_IN_L, LIP_IN_W = IN_L - 2 * (FIT + LIP_T), IN_W - 2 * (FIT + LIP_T)
assert LIP_IN_L >= MOSFET[0] + BUCK[0] + 1 and LIP_IN_W >= MOSFET[1], "modules do not fit inside the lip"

# Holes just below the lip, beside the MOSFET module's terminals.
HOLE_Z = BODY_H - LIP - CABLE_D / 2 - 1.0
assert HOLE_Z - CABLE_D / 2 > WALL + PACK_W, "cable holes would hit the board stack"

body = (
    cq.Workplane("XY")
    .box(L, W, BODY_H, centered=(True, True, False))
    .edges("|Z")
    .fillet(R)
    .faces(">Z")
    .shell(-WALL)
)
holes = (
    cq.Workplane("YZ", origin=(-L / 2 - 1, 0, 0))
    .pushPoints([(-CABLE_GAP / 2, HOLE_Z), (CABLE_GAP / 2, HOLE_Z)])
    .circle(CABLE_D / 2)
    .extrude(WALL + 2)
)
body = body.cut(holes)

plate = cq.Workplane("XY").box(L, W, LID, centered=(True, True, False)).edges("|Z").fillet(R)
lip = (
    cq.Workplane("XY")
    .box(IN_L - 2 * FIT, IN_W - 2 * FIT, LIP, centered=(True, True, False))
    .edges("|Z")
    .fillet(R - WALL)  # the body's inside corners
    .cut(cq.Workplane("XY").box(LIP_IN_L, LIP_IN_W, LIP, centered=(True, True, False)))
    .translate((0, 0, LID))
)
lid = plate.union(lip)

out = Path(__file__).resolve().parent / "export"
out.mkdir(exist_ok=True)
for name, part in {"body": body, "lid": lid}.items():
    for ext in ("stl", "step"):
        exporters.export(part, str(out / f"enclosure_v1_{name}.{ext}"))

print(f"inside {IN_L:.1f} x {IN_W:.1f} x {IN_H:.1f} mm, cable holes at z = {HOLE_Z:.1f} mm")
