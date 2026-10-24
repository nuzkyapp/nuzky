import json, sys
import numpy as np
d = json.load(open(sys.argv[1]))
for tr, e in d.items():
    ex = np.array(e["exact"])
    keep = (ex > 1) & (ex < 254)
    for k in ("conv_own", "conv_scaled"):
        c = np.array(e[k]); diff = (c - ex)[keep]
        print(f"{tr} {k}: mean {diff.mean():+.2f} mae {np.abs(diff).mean():.2f} max {np.abs(diff).max():.1f} | per channel {[round(float((c-ex)[...,i][keep[...,i]].mean()),2) for i in range(3)]}")
