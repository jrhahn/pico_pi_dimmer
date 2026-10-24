"""Enclosure v1 for the v3 dimmer: Pico + Pimoroni Wireless Pack, MOSFET
module and 24 V -> 5 V buck converter in one 60 x 25 x 47 mm box. The lid is
one of the two big 60 x 47 faces. Both cables (24 V in, LED strip out) leave
through the same end face.

Two parts, both print without support:

    enclosure_v1_body   open tray, floor on the bed. M2.5 heat-set inserts go
                        into the four bosses in its corners.
    enclosure_v1_lid    flat plate, screwed on with four M2.5 x 6 screws

Layout, looking into the open body:

    The Pico + pack stack lies flat on the floor, 5.5 mm off one long wall so
    the bosses clear it. The buck converter stands on edge in that gap, the
    MOSFET module on edge along the other side of the stack. Wire the module
    before it goes in: on edge, its terminal screws face sideways. The two
    cable holes face the module's end. Cinch a cable tie round each cable
    inside the wall as strain relief.

Modelled lid side up: x along the box, y across the lid, z through the lid.

Run in the CAD shell:  nix develop .#cad  &&  python cad/enclosure.py
"""

from math import copysign, sqrt
from pathlib import Path

import cadquery as cq
from cadquery import exporters

# Outside: the lid is L x W, H is the depth with the lid.
L, W, H = 60.0, 47.0, 25.0
WALL = 1.6  # walls and floor
LID = 1.6  # lid plate
R = 2.0  # corner radius of the lid's outline

# M2.5 heat-set inserts in the body hold the lid. ruthex M2.5x5.7 and CNC
# Kitchen M2.5x4 both want a 4.0 mm hole; check the datasheet of yours.
INSERT_D = 4.0  # hole for the insert
INSERT_L = 5.7  # insert length; the hole is 1 mm deeper for the screw tip
SCREW_D = 2.9  # clearance hole in the lid
SCREW_L = 6.0  # M2.5 x 6, pan or socket head

CABLE_D = 5.5  # cable hole diameter: the cable's outside + ~0.5
CABLE_WEB = 2.0  # wall left between the two holes

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

# Across the box: the buck on edge in the gap the bosses need, the stack, then
# the MOSFET module on edge.
GAP = BOSS_REACH + 0.5
PACK_EDGE = -IN_W / 2 + GAP + PACK_W  # the stack's side towards the module

# Cable holes in the end wall, between the stack and the bosses.
HOLE_Y = (PACK_EDGE + IN_W / 2 - BOSS_REACH) / 2
HOLES_Z = (BODY_H / 2 - (CABLE_D + CABLE_WEB) / 2, BODY_H / 2 + (CABLE_D + CABLE_WEB) / 2)

assert IN_L >= PACK_L + 1 and IN_H >= STACK + 1, "board stack does not fit"
assert IN_W >= GAP + PACK_W + 0.5 + MOSFET[2], "no room across the box for the MOSFET module"
assert GAP >= BUCK[2] + 1, "no room beside the stack for the buck"
assert 2 * (BOSS_X - BOSS_R) >= max(MOSFET[0], BUCK[0]) + 1, "modules do not fit between the bosses"
assert IN_H >= max(MOSFET[1], BUCK[1]) + 1, "modules do not fit on edge"
assert IN_W / 2 - BOSS_REACH - PACK_EDGE >= CABLE_D + 1, "no room for the cable holes"
assert HOLES_Z[0] - CABLE_D / 2 >= WALL + 1 and HOLES_Z[1] + CABLE_D / 2 <= BODY_H - 1, "cable holes too big"
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
holes = (
    cq.Workplane("YZ", origin=(-L / 2 - 1, 0, 0))
    .pushPoints([(HOLE_Y, z) for z in HOLES_Z])
    .circle(CABLE_D / 2)
    .extrude(WALL + 2)
)
body = body.cut(inserts).cut(holes)

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
    f"stack {GAP:.1f} mm off the wall, cable holes {HOLES_Z[0]:.1f} and {HOLES_Z[1]:.1f} mm above the bed"
)
