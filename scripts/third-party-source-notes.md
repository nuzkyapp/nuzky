The source code of the third-party libraries inside the [Nuzky vVERSION](https://github.com/nuzkyapp/nuzky/releases/tag/vVERSION) installers, as the GPL asks. You don't need any of this to use Nuzky.

- `build-info-*.txt`: what each installer was built from, with the exact FFmpeg and library versions.
- `…-third-party-source-linux.tar.gz`: the Ubuntu 24.04 source packages of the libraries in the AppImage.
- `…-third-party-source-macos.tar.gz`: the Homebrew formulas, sources and patches of the libraries in the macOS app.
- `…-third-party-source-windows.tar.gz`: FFmpeg at the commit the Windows build used, the FFmpeg-Builds scripts and the sources they pin.
- `…-third-party-source-onnxruntime.tar.gz`: ONNX Runtime with its dependencies.

Nuzky's own source is the Source code archive of the vVERSION release. Each archive has a README that says how it matches the build.
