# Instalación con binarios precompilados

Este directorio contiene dos templates xbps para Halley:

- **`template`** — compila desde source (requiere Rust/Cargo local, pesado)
- **`template-prebuilt`** — descarga binarios desde GitHub Releases (~100x más rápido)

## Flujo de trabajo

### 1. Publicar release en GitHub

```bash
# En tu fork local
git tag v0.5.0-mikuri.2
git push origin v0.5.0-mikuri.2
```

GitHub Actions (`.github/workflows/build-release.yml`) compila automáticamente:
- Binarios: `halley`, `halleyctl`, `xdg-desktop-portal-halley`
- Archivos de sistema: `.desktop`, portals config, D-Bus service
- Empaqueta todo en `halley-binaries-x86_64.tar.gz`
- Publica en GitHub Release con checksum SHA256

### 2. Actualizar template-prebuilt

Después que GH Actions complete:

1. Descargar checksum del release:
   ```bash
   curl -L https://github.com/mikuri12/halley/releases/download/v0.5.0-mikuri.2/halley-binaries-x86_64.tar.gz.sha256
   ```

2. Copiar el hash SHA256 en `template-prebuilt`:
   ```bash
   checksum="<pegar_hash_aqui>"
   ```

3. Actualizar `version` y `revision` si cambiaron:
   ```bash
   version=0.5.0
   revision=2  # incrementar con cada rebuild del mismo version
   ```

### 3. Instalar en Void Linux

```bash
# Copiar template-prebuilt al repo xbps-src
cp template-prebuilt ~/void-packages/srcpkgs/halley/template

# Compilar paquete (solo descarga + empaqueta, no compila Rust)
cd ~/void-packages
./xbps-src pkg halley

# Instalar
sudo xbps-install --repository hostdir/binpkgs halley
```

## Ventajas vs compilación local

| Aspecto | Local (`template`) | Prebuilt (`template-prebuilt`) |
|---------|-------------------|-------------------------------|
| Tiempo instalación | ~20-40 min | ~30 segundos |
| Deps build | rust, cargo, clang18, 10+ libs | ninguna |
| Uso disco | +2GB (cargo registry + target/) | ~15MB |
| Reproducibilidad | depende de rustc local | binarios idénticos |

## Verificación de deps runtime

`template-prebuilt` verifica automáticamente las dependencias runtime:
- xwayland-satellite, dbus, seatd
- wayland, libxkbcommon, libinput, libseat
- libudev-zero, libgbm, libdrm, libglvnd
- pixman, pipewire

Si falta algo, xbps lo instalará automáticamente.

## Troubleshooting

**Error: "checksum mismatch"**
→ Hash SHA256 en template-prebuilt no coincide. Reemplazar con el del release.

**Error: "404 Not Found"**
→ Release tag no existe. Verificar que GH Actions completó exitosamente.

**Binarios no ejecutan**
→ Verificar arquitectura (`uname -m` debe ser `x86_64`) y que las libs runtime estén instaladas.
