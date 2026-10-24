# Compares Nuzky's drawn HDR patches (old and exact input) with Apple's avconvert SDR output of the same grid.
import json, subprocess, sys
import numpy as np
dump, apple_mov = sys.argv[1], sys.argv[2]
P, COLS = 16, 68
d = json.load(open(dump))["arib-std-b67"]
n = len(d["signal"])
info = json.loads(subprocess.run(["ffprobe", "-v", "error", "-select_streams", "v:0", "-show_streams", "-of", "json", apple_mov], capture_output=True, check=True).stdout)["streams"][0]
w, h, fmt = info["width"], info["height"], info["pix_fmt"]
print("apple:", info["codec_name"], fmt, w, h, info.get("color_range"), info.get("color_space"), info.get("color_transfer"), info.get("color_primaries"))
assert fmt in ("yuv420p", "yuvj420p"), fmt
raw = subprocess.run(["ffmpeg", "-v", "error", "-i", apple_mov, "-frames:v", "1", "-f", "rawvideo", "-pix_fmt", fmt, "-"], capture_output=True, check=True).stdout
y = np.frombuffer(raw[: w * h], np.uint8).reshape(h, w).astype(float)
u = np.frombuffer(raw[w * h : w * h * 5 // 4], np.uint8).reshape(h // 2, w // 2).astype(float)
v = np.frombuffer(raw[w * h * 5 // 4 : w * h * 3 // 2], np.uint8).reshape(h // 2, w // 2).astype(float)
full = info.get("color_range") == "pc" or fmt == "yuvj420p"
apple = []
for i in range(n):
    x0, y0 = (i % COLS) * P, (i // COLS) * P
    yy = y[y0 + 4 : y0 + 12, x0 + 4 : x0 + 12].mean()
    cb = u[(y0 + 4) // 2 : (y0 + 12) // 2, (x0 + 4) // 2 : (x0 + 12) // 2].mean()
    cr = v[(y0 + 4) // 2 : (y0 + 12) // 2, (x0 + 4) // 2 : (x0 + 12) // 2].mean()
    if full:
        Y, Pb, Pr = yy / 255, (cb - 128) / 255, (cr - 128) / 255
    else:
        Y, Pb, Pr = (yy - 16) / 219, (cb - 128) / 224, (cr - 128) / 224
    r, b = Y + 1.5748 * Pr, Y + 1.8556 * Pb
    g = (Y - 0.2126 * r - 0.0722 * b) / 0.7152
    apple.append(np.clip(np.array([r, g, b]) * 255, 0, 255))
apple = np.array(apple)
np.save(apple_mov + ".npy", apple)
groups = {"grid 17^3": slice(0, 4913), "8 patches": slice(4913, 4921), "skin ramp": slice(4921, 4929), "Monk": slice(4929, 4939), "Monk 2x light": slice(4939, 4949), "all": slice(0, n)}
for label in ("old", "new"):
    drawn = np.array(d[f"drawn_{label}"])
    diff = drawn - apple
    print(f"{label:4}", "  ".join(f"{g}: mae {np.abs(diff[s]).mean():.2f} mean {diff[s].mean():+.2f}" for g, s in groups.items()))
old, new = np.array(d["drawn_old"]), np.array(d["drawn_new"])
closer = np.abs(new - apple).mean(axis=1) < np.abs(old - apple).mean(axis=1)
farther = np.abs(new - apple).mean(axis=1) > np.abs(old - apple).mean(axis=1)
print(f"patches where the exact input is closer to Apple: {closer.sum()}, farther: {farther.sum()}, of {n}")
print("patch | signal | Apple | old | exact input")
for i in list(range(4913, n)):
    print(i, np.round(d["signal"][i], 3), np.round(apple[i]).astype(int), np.round(old[i]).astype(int), np.round(new[i]).astype(int))
