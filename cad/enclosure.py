"""Enclosure v1 for the v3 dimmer: Pico + Pimoroni Wireless Pack, MOSFET
module and 24 V -> 5 V buck converter in one 80 x 67 x 45 mm box with the lid
on top. Both cables (24 V in, LED strip out) leave through slots in the same
end face. The slots are open at the rim, so cables with their plugs on go in
from above; the lid closes them.

Two parts, both print without support:

    enclosure_v1_body   open tray, floor on the bed. M2.5 heat-set inserts go
                        into the four bosses in its corners.
    enclosure_v1_lid    flat plate, screwed on with four M2.5 x 6 screws

Inside, everything lies flat on the floor on double-sided foam tape: the
Pico + pack stack, the MOSFET module with its terminals up, the buck
converter. Keep the corners free for the bosses. Cinch a cable tie round each
cable inside the wall as strain relief.

Run in the CAD shell:  nix develop .#cad  &&  python cad/enclosure.py
"""

from math import copysign, sqrt
from pathlib import Path

import cadquery as cq
from cadquery import exporters

# Outside: the lid is L x W, H is the height with the lid.
L, W, H = 80.0, 67.0, 45.0
WALL = 1.6  # walls and floor
LID = 1.6  # lid plate
R = 2.0  # corner radius of the lid's outline

# M2.5 heat-set inserts in the body hold the lid. ruthex M2.5x5.7 and CNC
# Kitchen M2.5x4 both want a 4.0 mm hole; check the datasheet of yours.
INSERT_D = 4.0  # hole for the insert
INSERT_L = 5.7  # insert length; the hole is 1 mm deeper for the screw tip
SCREW_D = 2.9  # clearance hole in the lid
SCREW_L = 6.0  # M2.5 x 6, pan or socket head

CABLE_D = 5.5  # cable slot width: the thicker cable's outside + ~0.5
CABLE_GAP = 11.0  # slot centre to centre

# What has to fit (mm). STACK is the Pico + pack sandwich, plugged together,
# from the pack's top components to the Pico's -- measure yours.
PACK_L, PACK_W, STACK = 53.0, 25.0, 18.0
MOSFET = (34.0, 17.0, 12.0)  # dual-MOSFET trigger module incl. screw terminals
BUCK = (17.2, 11.2, 4.0)  # Mini-360

IN_L, IN_W = L - 2 * WALL, W - 2 * WALL
BODY_H = H - LID
IN_H = BODY_H - WALL  # floor to the underside of the lid

# Corner bosses: 1.2 mm of plastic round the insert, 1.4 mm between it and the
# outside. Below the insert the boss runs at 45 degrees into the corner until
# it is buried in the walls, so it needs no support.
BOSS_R = INSERT_D / 2 + 1.2
BOSS_X, BOSS_Y = L / 2 - INSERT_D / 2 - 1.4, W / 2 - INSERT_D / 2 - 1.4
BOSS_H = INSERT_L + 2.0  # straight part: insert hole + 1 mm floor
BOSS_REACH = IN_L / 2 - BOSS_X + BOSS_R  # how far a boss reaches in from the walls
SLANT = BOSS_R + sqrt(2) * (IN_L / 2 - BOSS_X)
CORNERS = [(sx * BOSS_X, sy * BOSS_Y) for sx in (-1, 1) for sy in (-1, 1)]

# Each slot just deep enough for its cable, so the lid holds it.
SLOT_BOTTOM = BODY_H - CABLE_D

assert IN_L >= PACK_L + 2 * BOSS_REACH and IN_H >= STACK + 1, "board stack does not fit"
assert IN_W >= PACK_W + MOSFET[1] + BUCK[1] + 3, "modules do not fit beside the stack"
assert SCREW_L - LID <= INSERT_L + 1, "screws too long for the insert holes"


def boss(x, y):
    """Straight down from the rim round the insert, then 45 degrees into the corner."""
    base = cq.Vector(x, y, BODY_H - BOSS_H)
    disc = cq.Face.makeFromWires(cq.Wire.makeCircle(BOSS_R, base, cq.Vector(0, 0, 1)))
    run = SLANT / sqrt(2)
    slant = cq.Solid.extrudeLinear(disc, cq.Vector(copysign(run, x), copysign(run, y), -SLANT))
    return cq.Solid.makeCylinder(BOSS_R, BOSS_H, base).fuse(slant)


outline = cq.Workplane("XY").box(L, W, BODY_H, centered=(True, True, False)).edges("|Z").fillet(R)
body = outline.faces(">Z").shell(-WALL)
for x, y in CORNERS:
    body = body.union(boss(x, y))
body = body.intersect(outline)  # the slants run out through the walls
inserts = (
    cq.Workplane("XY", origin=(0, 0, BODY_H - INSERT_L - 1))
    .pushPoints(CORNERS)
    .circle(INSERT_D / 2)
    .extrude(INSERT_L + 1)
)
slots = (
    cq.Workplane("YZ", origin=(-L / 2 - 1, 0, 0))
    .pushPoints([(s * CABLE_GAP / 2, SLOT_BOTTOM + CABLE_D / 2 + 1) for s in (-1, 1)])
    .slot2D(CABLE_D + 2, CABLE_D, 90)  # round bottom, top end past the rim
    .extrude(WALL + 2)
)
body = body.cut(inserts).cut(slots)

lid = (
    cq.Workplane("XY")
    .box(L, W, LID, centered=(True, True, False))
    .edges("|Z")
    .fillet(R)
    .faces(">Z")
    .workplane()
    .pushPoints(CORNERS)
    .hole(SCREW_D)
)

out = Path(__file__).resolve().parent / "export"
out.mkdir(exist_ok=True)
for name, part in {"body": body, "lid": lid}.items():
    for ext in ("stl", "step"):
        exporters.export(part, str(out / f"enclosure_v1_{name}.{ext}"))

print(
    f"inside {IN_L:.1f} x {IN_W:.1f} x {IN_H:.1f} mm, "
    f"bosses reach down to {BODY_H - BOSS_H - SLANT:.1f} mm above the bed"
)
