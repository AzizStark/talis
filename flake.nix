{
  description = "Talis desktop turntable";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";

  outputs =
    { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = nixpkgs.legacyPackages.${system};
      talis = pkgs.callPackage ./packaging/nix/package.nix { };
    in
    {
      packages.${system} = {
        default = talis;
        inherit talis;
      };
      apps.${system}.default = {
        type = "app";
        program = "${talis}/bin/talis";
        meta.description = "An interactive, animated and music-aware desktop turntable";
      };
      checks.${system}.package = pkgs.callPackage ./packaging/nix/check.nix { inherit talis; };
    };
}
