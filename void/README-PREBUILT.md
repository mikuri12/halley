# Instalación con binarios precompilados

Este directorio trae dos templates xbps para Halley:

- **`template`** — compila desde fuente (necesita Rust/Cargo local, lento)
- **`template-prebuilt`** — *plantilla* del que descarga binarios ya compilados

`template-prebuilt` **no se usa directamente**: lleva los placeholders `@TAG@` y
`@CHECKSUM@`. GitHub Actions los sustituye y publica el resultado como asset
`template` del release. Ese es el que se descarga.

## Instalar (lo único que hace falta normalmente)

```sh
git clone --depth=1 https://github.com/void-linux/void-packages.git
cd void-packages
./xbps-src binary-bootstrap

mkdir -p srcpkgs/halley
curl -L -o srcpkgs/halley/template \
  https://github.com/mikuri12/halley/releases/download/v0.5.0-mikuri.2/template

./xbps-src pkg halley
doas xbps-install --repository hostdir/binpkgs halley
```

El template del release ya trae el checksum correcto: no hay que editar nada.
`./xbps-src pkg` no compila Rust — descarga el tarball, verifica el SHA256 y
empaqueta.

## Publicar una versión nueva

```sh
git tag v0.5.0-mikuri.3
git push origin v0.5.0-mikuri.3
```

El workflow `.github/workflows/build-release.yml` hace el resto:

1. Compila el workspace **dentro de un contenedor Void glibc**
   (`ghcr.io/void-linux/void-glibc-full`), no en el runner Ubuntu.
2. Empaqueta binarios + `.desktop` + metadata de portals + servicio D-Bus en
   `halley-binaries-<tag>-x86_64.tar.gz`.
3. Calcula el SHA256 y lo inyecta en `template-prebuilt` → asset `template`.
4. Publica tarball, `.sha256` y `template` en el release.

**No muevas un tag ya publicado.** GitHub regenera el tarball y el SHA256 cambia;
es lo que rompía el `template` de fuente antes. Para una versión nueva, tag nuevo.

### Por qué el contenedor Void y no Ubuntu

Un binario linkeado en Ubuntu no arranca en Void: las sonames no coinciden
(`libinput.so.10`, `libseat.so.1`, …) y la glibc es otra. Compilando dentro de la
imagen de Void, los binarios linkean contra las mismas libs que el sistema
destino.

Se usa `docker run` en vez de la clave `container:` de Actions porque la imagen
de Void no trae `node` y `actions/checkout` no podría ejecutarse.

## Comparación

| Aspecto | `template` (fuente) | `template` del release (prebuilt) |
|---------|--------------------|-----------------------------------|
| Tiempo | decenas de minutos | segundos |
| Deps de build | rust, cargo, clang18 + ~14 `-devel` | ninguna |
| Disco | ~2GB (`target/` + registry) | ~15MB |
| Reproducible | depende del rustc local | binario idéntico para todos |

## Dependencias de runtime

El template solo declara lo que xbps no puede deducir del ELF:

```
depends="xwayland-satellite dbus seatd"
```

Las libs compartidas (wayland, libxkbcommon, libinput, libseat, libudev, libgbm,
libdrm, libglvnd, pixman, pipewire) las detecta `xbps-src` escaneando los
binarios y las añade como `shlib-requires`. Declararlas a mano solo sirve para
equivocarse de nombre. `xbps-install` las instala si faltan.

## Troubleshooting

**`checksum mismatch`**
→ Descargaste `void/template-prebuilt` del repo en vez del asset `template` del
release. El del repo lleva `@CHECKSUM@` sin sustituir.

**`404 Not Found` al bajar el distfile**
→ El release no existe o el workflow falló. Mirar la pestaña Actions.

**`ERROR: Package 'halley' not found in repository pool'`**
→ `./xbps-src pkg halley` falló antes; el error real está más arriba en su
salida. `xbps-install` no tiene nada que instalar.

**El binario no arranca / falta un `.so`**
→ Comprobar que el paso "Build inside Void container" del workflow corrió.
