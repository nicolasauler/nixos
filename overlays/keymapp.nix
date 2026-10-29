# keymapp is ZSA's prebuilt binary. autoPatchelfHook points it at Nix's dynamic
# loader and libraries with the first patchelf on the build's PATH: first the
# loader, then the library path. With patchelf 0.19.2, nixpkgs' since 2026-10,
# that leaves the binary's .dynamic table in a read-only segment, and keymapp
# segfaults in ld.so before it starts (nixpkgs#578860, NixOS/patchelf#674 and
# #675; the merged patchelf#672 doesn't fix this binary). patchelf 0.15.2,
# nixpkgs' version until then, patches it correctly, so keymapp is built with
# that one. Once nixpkgs' patchelf changes (another version, or patches on
# 0.19.2), evaluation fails until someone checks whether keymapp still crashes
# when built with it.
final: prev: {
  keymapp = prev.keymapp.overrideAttrs (old: {
    nativeBuildInputs =
      (
        if prev.patchelf.version == "0.19.2" && (prev.patchelf.patches or []) == []
        then [
          (final.stdenv.mkDerivation {
            pname = "patchelf";
            version = "0.15.2";
            src = final.fetchurl {
              url = "https://github.com/NixOS/patchelf/releases/download/0.15.2/patchelf-0.15.2.tar.bz2";
              hash = "sha256-F3RfVkFZyOIo/EEtplogSLhGxLa0Igt3y/IkFuAvLXw=";
            };
          })
        ]
        else throw "nixpkgs' patchelf changed (version ${prev.patchelf.version}, patches: ${toString (builtins.length (prev.patchelf.patches or []))}): check whether keymapp built with it still segfaults at startup (nixpkgs#578860); if not, drop overlays/keymapp.nix"
      )
      ++ old.nativeBuildInputs;
  });
}
