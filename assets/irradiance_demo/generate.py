"""Generate an authored SH9 demonstration, not a light bake or capture."""
import json
from pathlib import Path

nodes = []
# x-fastest uniform grid; every band is deliberately populated with signed RGB.
for z in range(4):
    for y in range(4):
        for x in range(4):
            energy = [0.65 + 0.22*x, 0.45 + 0.18*y, 0.55 + 0.20*z]
            nodes.append([
                [round(e / 0.2820948, 7) for e in energy],
                [0.10, -0.04, 0.06], [-0.03, 0.07, 0.12], [0.14, 0.02, -0.06],
                [0.025, -0.012, 0.018], [-0.015, 0.010, 0.025],
                [0.030, -0.020, 0.012], [-0.018, 0.023, 0.014], [0.022, 0.011, -0.017],
            ])

grid = {
    "version": 1, "enabled": True, "provenance": "authored",
    "dimensions": [4, 4, 4], "origin": [-4.5, -3.0, -4.5], "spacing": [3.0, 3.0, 3.0],
    "coefficients": nodes,
}
Path(__file__).with_name("authored-room.irradiance.json").write_text(json.dumps(grid, indent=2) + "\n")
