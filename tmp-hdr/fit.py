import json, sys
import numpy as np
d = json.load(open(sys.argv[1]))
L2020 = np.array([0.2627, 0.678, 0.0593])
M = np.array([[1.6605, -0.5876, -0.0728], [-0.1246, 1.1329, -0.0083], [-0.0182, -0.1006, 1.1187]])
def pq_to_nits(e):
    p = e ** (1 / 78.84375)
    return 10000 * (np.maximum(p - 0.8359375, 0) / (18.8515625 - 18.6875 * p)) ** (1 / 0.1593017578125)
def tone(rgb255, power, toe, pq):
    e = np.clip(rgb255 / 255.0, 0, 1)
    if pq:
        disp = pq_to_nits(e) / 1000
        scene = disp * np.maximum(disp @ L2020, 1e-6)[..., None] ** (-1 / 6)
    else:
        scene = np.where(e <= 0.5, e * e / 3, (np.exp((e - 0.55991073) / 0.17883277) + 0.28466892) / 12)
    y = np.maximum(scene @ L2020, 1e-6)
    lum = y ** power
    sdr = np.clip((scene * (lum / y * lum / (lum + toe))[..., None]) @ M.T, 0, 1)
    return np.round(255 * sdr ** (1 / 2.4))
P0, T0 = 0.93, 0.0014
for tr, e in d.items():
    pq = tr == "smpte2084"
    ex, own, drawn = (np.array(e[k]) for k in ("exact", "conv_own", "drawn"))
    model = tone(own, P0, T0, pq)
    print(f"{tr}: model of old pipeline vs renderer: mean {np.mean(model - drawn):+.2f} mae {np.mean(np.abs(model - drawn)):.2f} max {np.max(np.abs(model - drawn)):.1f}")
    target = model
    base = tone(ex, P0, T0, pq)
    print(f"  exact input, old constants vs old pipeline: mean {np.mean(base - target):+.2f} mae {np.mean(np.abs(base - target)):.2f}")

APPLE = np.array([[55.0]*3, [98.0]*3, [155.0]*3, [190.0]*3, [232.0]*3, [131, 102, 93], [198, 180, 150], [177, 25, 40]])
e = d["arib-std-b67"]
ex, own = np.array(e["exact"]), np.array(e["conv_own"])
target = tone(own, P0, T0, False)
best = None
for power in np.arange(0.880, 0.981, 0.0025):
    for toe in np.arange(0.0, 0.0041, 0.0001):
        err = np.mean(np.abs(tone(ex, power, toe, False) - target))
        if best is None or err < best[0]:
            best = (err, power, toe)
print("best", best)
err, power, toe = best
for p, t in [(P0, T0), (power, toe)]:
    new = tone(ex, p, t, False)
    grid, apple8, skin = new[:4913], new[4913:4921], new[4921:]
    print(f"power {p:.4f} toe {t:.4f}: vs old pipeline mean {np.mean(new-target):+.2f} mae {np.mean(np.abs(new-target)):.2f} (grid {np.mean(np.abs(grid-target[:4913])):.2f}, skin ramp {np.mean(np.abs(skin-target[4921:])):.2f}); vs Apple 8 patches mae {np.mean(np.abs(apple8-APPLE)):.2f} max {np.max(np.abs(apple8-APPLE)):.0f}")
old8 = target[4913:4921]
print(f"old pipeline vs Apple 8 patches mae {np.mean(np.abs(old8-APPLE)):.2f} max {np.max(np.abs(old8-APPLE)):.0f}")

print("patch  apple | old pipeline | exact+old consts | exact+refit")
refit = tone(ex, power, toe, False)[4913:4921]
base8 = tone(ex, P0, T0, False)[4913:4921]
for i in range(8):
    print(i, APPLE[i], old8[i], base8[i], refit[i])
best8 = None
for p in np.arange(0.85, 1.01, 0.0025):
    for t in np.arange(0.0, 0.006, 0.0001):
        err = np.mean(np.abs(tone(ex[4913:4921], p, t, False) - APPLE))
        if best8 is None or err < best8[0]: best8 = (err, p, t)
print("best fit to Apple's 8 patches with exact input", best8)
best8o = None
for p in np.arange(0.85, 1.01, 0.0025):
    for t in np.arange(0.0, 0.006, 0.0001):
        err = np.mean(np.abs(tone(own[4913:4921], p, t, False) - APPLE))
        if best8o is None or err < best8o[0]: best8o = (err, p, t)
print("best fit to Apple's 8 patches with darker input", best8o)
