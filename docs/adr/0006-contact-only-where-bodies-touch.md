# ADR 0006: Contact only where bodies touch

Status: accepted (2026-11-17)

## Context

Each body has its own velocity field on the grid, and bodies were made to
interact at every node that carried mass of more than one of them. A node
carries mass of a body as soon as one of its particles is within a cell
and a half, so two bodies interacted across a gap of up to three cells.

For a full-caliber shell this is not a detail. A 152 mm shell striking
thick steel head-on at 810 m/s reached 97, 118, 132 and 147 mm at 8, 12, 16
and 20 cells per radius: the depth grew by the same amount with every
refinement and showed no sign of settling. The gap acts like a thicker
shell, by about two cells on the radius, and that error shrinks only in
proportion to the cell size.

A second, unrelated error hid the first: the depth was taken from the
foremost projectile particle, and one or two failed, strengthless particles
squeezed far ahead of the shell added tens of millimetres.

## Decision

- **Bodies at a node are in contact only if they touch.** A node already
  holds, for each body, its mass and the kernel-weighted position of its
  centre of mass (the vector used for the contact normal). For two bodies
  that fill the space on either side of a plane, the distance between
  those centres along the normal is a closed-form function of how the
  node's mass is shared between them (`rap_solver::contact`); it is 0.8125
  cells when they share it equally. If the centres are further apart than
  that, plus a margin of 0.02 cells, there is a gap and the contact
  constraint is not applied. All four solvers use the same test.
- **Depth is that of intact material, or of a hundredth of the mass.** The
  penetration depth is the foremost intact projectile material, or the
  position beyond which one percent of the projectile's mass lies, failed
  or not, whichever is further. Stray failed particles no longer count;
  the debris of an eroding rod at the bottom of its crater still does.
- **Depth is the greatest reached.** A shell that is stopped is thrown back
  out of its crater; the reported depth is the largest found at any time
  the simulation was advanced to.

## Consequences

- The same head-on shot now reaches 127, 148, 166, 166 and 170 mm at 6, 8,
  12, 16 and 20 cells per radius: it has settled by 12 cells per radius.
- Two elastic bars colliding at 20 m/s now exchange 91 % of the striker's
  momentum (struck bar 18.24 m/s) instead of 90 %, and separate.
- The test is exact for a flat interface along a grid axis. Along other
  directions the touching distance differs by about two percent, and where
  the smaller body holds less than about two percent of a node's mass a gap
  of a tenth of a cell can no longer be told from contact.
- Results of every earlier run with two bodies change; the validation
  numbers in the README were measured again.
