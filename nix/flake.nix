{
  description = "Halley-Mikuri: patched Halley Wayland compositor (v0.5.0 + local fixes)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    home-manager = {
      url = "github:nix-community/home-manager";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = {
    self,
    nixpkgs,
    flake-utils,
    home-manager,
  }:
    flake-utils.lib.eachDefaultSystem (system: let
      pkgs = import nixpkgs {inherit system;};
      halley = pkgs.callPackage ./halley.nix {};
    in {
      packages = {
        inherit halley;
        default = halley;
      };

      # Ready-to-use Home Manager configuration for a `mikuri` user.
      # Apply with: home-manager switch --flake github:mikuri12/halley#mikuri
      # (or replace "mikuri" with your own username by overriding the module).
      homeConfigurations.mikuri = home-manager.lib.homeManagerConfiguration {
        inherit pkgs;
        modules = [
          ({config, pkgs, ...}: {
            home.username = "mikuri";
            home.homeDirectory = "/home/mikuri";
            home.stateVersion = "25.05";
            programs.home-manager.enable = true;
            # standalone HM on non-NixOS distros (Void, Arch, etc.)
            targets.genericLinux.enable = true;

            home.packages = [halley];

            # Expose the Wayland session so the greeter (ly/Noctalia/SDDM)
            # offers "Halley" in its menu. In Void's greeter reads
            # /usr/share/wayland-sessions, NOT ~/.local/share -- so you'll
            # need to either symlink it from /usr/share or run Halley from
            # the .desktop that the xbps package installs (see ../void/).
            xdg.dataFile."wayland-sessions/halley.desktop".source =
              "${halley}/share/wayland-sessions/halley.desktop";

            # Native portal backend metadata so xdg-desktop-portal picks up
            # the halley ScreenCast/Screenshot (dmabuf zero-copy).
            xdg.dataFile."xdg-desktop-portal/portals/halley.portal".source =
              "${halley}/share/xdg-desktop-portal/portals/halley.portal";
            xdg.dataFile."xdg-desktop-portal/halley-portals.conf".source =
              "${halley}/share/xdg-desktop-portal/halley-portals.conf";

            # User-systemd unit (no-op on Void runit, useful on NixOS).
            xdg.configFile."systemd/user/halley.service".source =
              "${halley}/lib/systemd/user/halley.service";
            xdg.configFile."systemd/user/halley-shutdown.target".source =
              "${halley}/lib/systemd/user/halley-shutdown.target";
          })
        ];
      };
    });
}
