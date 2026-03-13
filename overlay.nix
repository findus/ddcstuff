final: prev: {
  ddcstuff = final.rustPlatform.buildRustPackage {
    pname = "ddcstuff";
    version = "0.1.0";

    src = ./.;

    cargoLock.lockFile = ./Cargo.lock;

    nativeBuildInputs = [
      final.pkg-config
    ];

    buildInputs = [
      final.udev        # libudev — required by tokio-udev / libudev-sys
      final.linuxHeaders # i2c kernel headers — required by i2c-linux-sys
    ];

    meta = with final.lib; {
      description = "DDC/CI brightness control daemon for Linux";
      homepage = "https://github.com/findus/ddc";
      license = licenses.mit;
      maintainers = [ ];
      platforms = [ "x86_64-linux" "aarch64-linux" ];
    };
  };
}
