# Read local path-info metadata only; never evaluate a package or source flake.
let
  metadata = builtins.fromJSON (builtins.readFile (builtins.getEnv "TALIS_NIX_PATH_INFO"));
  entries =
    if builtins.isList metadata then
      metadata
    else
      map (path: metadata.${path} // { inherit path; }) (builtins.attrNames metadata);
  cached = entry: builtins.any
    (signature: builtins.match "cache[.]nixos[.]org-1:.*" signature != null)
    (entry.signatures or [ ]);
in
builtins.concatStringsSep "\n"
  (map (entry: "${if cached entry then "substitute" else "payload"} ${entry.path}") entries)
+ "\n"
