# Window Memory

Utilidad nativa de Windows escrita en Rust que recuerda el tamaño y la posición de
las ventanas de cada aplicación **en cada monitor**, y restaura esa geometría cuando
la ventana vuelve al monitor.

Es un único ejecutable Win32 sin frameworks gráficos ni runtime adicional: vive en la
bandeja del sistema, no tiene ventana principal y no sondea el escritorio.

## Ejemplo

Monitor principal 4K horizontal y monitor secundario vertical:

1. Chrome se usa en el principal con unas dimensiones determinadas.
2. Al arrastrarlo al vertical, Window Memory recuerda la geometría del principal y
   adapta la ventana al vertical (tamaño escalado por DPI y posición proporcional).
3. Al devolverlo al principal, recupera automáticamente su geometría original.

## Requisitos

- Rust estable con toolchain MSVC (`x86_64-pc-windows-msvc`), edition 2024.
- Windows 10 o superior.

## Primeros pasos

```powershell
cargo run                 # icono en la bandeja, sin ventana
cargo run -- --show       # abre el panel al arrancar
cargo run -- --dump       # imprime el estado en JSON y termina
cargo test
cargo build --release     # target\release\window-memory.exe
```

## Cómo funciona

- **Detección por eventos.** Se instalan hooks con `SetWinEventHook`
  (`EVENT_SYSTEM_MOVESIZESTART`, `EVENT_SYSTEM_MOVESIZEEND`,
  `EVENT_OBJECT_LOCATIONCHANGE`, `EVENT_OBJECT_DESTROY` y `EVENT_OBJECT_HIDE`) con
  `WINEVENT_OUTOFCONTEXT`. El callback solo publica un mensaje en la ventana oculta;
  el trabajo se hace en el hilo de la interfaz. No hay sondeos periódicos de ventanas.
- **Restauración al soltar, no durante el arrastre.** Mientras una ventana se
  arrastra no se toca. Al terminar el movimiento se identifica el monitor de destino:
  si hay geometría recordada se restaura, y si no, se calcula una adaptada y se guarda.
- **Sin bucles de eventos.** La geometría aplicada queda registrada como la actual, y
  durante 400 ms se descartan los eventos provocados por la propia restauración.
- **Geometrías relativas al área de trabajo** (`rcWork`) del monitor, no al escritorio
  virtual: sobreviven a cambios de la barra de tareas y a la disposición de monitores.
- **DPI por monitor.** El proceso es «per-monitor aware v2». Al adaptar a un monitor
  nuevo, el tamaño se escala por el cociente de DPI y el centro se mantiene
  proporcional. Al restaurar se encaja además en el área de trabajo de destino.
- **Cambios de monitores.** `WM_DISPLAYCHANGE` y `WM_SETTINGCHANGE` reenumeran los
  monitores y descartan el seguimiento de ventanas que quedaron en monitores retirados.

### Ventanas que no se siguen

Ventanas propias, hijas, invisibles, ocultas por DWM, con propietario, de
herramienta, cuadros de diálogo (`#32770`), maximizadas, minimizadas, con geometría
no válida y aplicaciones excluidas por configuración. La identidad de la aplicación es
su ejecutable sin extensión (`chrome.exe` → `chrome`), por lo que varias instancias de
una misma aplicación comparten la geometría recordada para cada monitor.

## Configuración

Todo se guarda en `%LOCALAPPDATA%\WindowMemory`:

| Archivo | Contenido |
| --- | --- |
| `config.toml` | Ajustes editables a mano. |
| `geometries.toml` | Geometrías recordadas; lo reescribe la aplicación. |
| `window-memory.log` | Registro rotativo (512 KB) con el ciclo de vida y los errores. |

```toml
# Intervalo de actualización del panel, en segundos (10-3600).
refresh_seconds = 60

# Lista vacía: se supervisan todas las aplicaciones menos las excluidas.
supervised = []
excluded = ["explorer"]

[general]
enabled = true
auto_restore = true

[applications.chrome]
enabled = true

[applications.code]
enabled = true

[applications.explorer]
enabled = false
```

- `general.enabled` apaga Window Memory por completo (también retira los hooks).
- `general.auto_restore` desactiva solo la restauración: las geometrías se siguen
  recordando.
- `supervised` restringe la lista de aplicaciones; `excluded` siempre gana.
- `applications.<app>` permite desactivar una aplicación concreta. Las claves admiten
  mayúsculas y `.exe`; se normalizan al comparar.

El menú de la bandeja y el diálogo de configuración guardan el archivo completo, así
que reescriben sus comentarios.

### Geometrías

```toml
version = 1

[[geometry]]
app = "chrome"
monitor = "display1"
x = 100
y = 80
width = 1600
height = 1000
maximized = false
```

`monitor` es una clave estable construida a partir del nombre de dispositivo
(`\\.\DISPLAY1` → `display1`). Si Windows no expone ese nombre se usa la geometría del
monitor como aproximación. Las coordenadas son relativas al área de trabajo.

## Interfaz

Icono en la bandeja del sistema:

- **Mostrar estado** (también con clic izquierdo): panel con el estado de la memoria.
- **Window Memory activo**: activa o desactiva la utilidad.
- **Restauración automática**: activa o desactiva la restauración al cambiar de monitor.
- **Abrir configuración**: diálogo con ambas casillas, el intervalo del panel y un
  botón para abrir la carpeta de datos.
- **Iniciar con Windows**, **Actualizar ahora**, **Probar notificación** y **Salir**.

## CLI

| Argumento | Efecto |
| --- | --- |
| `--show` | Abre el panel al arrancar. |
| `--dump` | Imprime el estado en JSON y termina. |
| `--version` | Muestra la versión y termina. |
| `--help` | Muestra la ayuda y termina. |

Sin argumentos, la aplicación solo deja el icono en la bandeja.

## Estructura

```
src/
├─ main.rs              Punto de entrada del ejecutable
├─ lib.rs               Módulos internos y `run()`
├─ cli.rs               Análisis de argumentos y salidas de consola
├─ app.rs               Estado, orquestación, hooks y procedimientos de ventana
├─ config.rs            Configuración TOML y reglas de supervisión
├─ meta.rs              Identidad: nombres, rutas, versión y log
├─ strings.rs           Textos de la interfaz
├─ tray.rs              Icono de bandeja y menú contextual
├─ panel.rs             Panel emergente (modelo de vista + pintado GDI)
├─ settings.rs          Diálogo modal de configuración
├─ window_manager.rs    Filtros, seguimiento de ventanas y decisiones de restauración
├─ window_memory.rs     Geometrías por aplicación y monitor, y su archivo TOML
└─ platform/            Capa Win32 reutilizable, sin lógica de la aplicación
   ├─ console.rs        Salida por consola desde un ejecutable gráfico
   ├─ dialog.rs         Controles, fuentes y avisos de Win32
   ├─ dpi.rs            Conciencia de DPI por monitor
   ├─ geometry.rs       Rectángulos en píxeles físicos
   ├─ instance.rs       Instancia única
   ├─ log.rs            Registro en archivo con rotación
   ├─ monitor.rs        Enumeración e identificación de monitores
   ├─ paths.rs          Rutas de `%LOCALAPPDATA%`
   ├─ registry.rs       Inicio automático con Windows
   ├─ string.rs         Conversión a UTF-16
   ├─ time.rs           Hora local
   └─ window.rs         Consultas de ventana y hooks de eventos
```

`platform` no importa módulos de la aplicación: solo depende del crate `windows`.

`window_manager::Manager` recibe hechos de ventana ya resueltos (`WindowFacts`) y
devuelve un `Outcome` con el rectángulo a aplicar, así que toda la lógica de decisión
se prueba sin tocar el escritorio real. El panel no sondea nada: `snapshot()` refleja
los contadores que mantiene el gestor.

## Pruebas

```powershell
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Las pruebas cubren:

- Identificación de monitores: claves estables por dispositivo y respaldo por geometría.
- Conversión de coordenadas relativas y absolutas, incluidos orígenes negativos.
- Guardado y recuperación de geometrías, con archivo TOML y con entradas corruptas.
- Ventanas sin geometría previa: adaptación por DPI y encaje en el área de trabajo.
- Cambios de resolución y disposición de monitores.
- Ventanas maximizadas, minimizadas, diálogos, ventanas hijas y de herramienta.
- Varias instancias de la misma aplicación.
- Prevención de bucles: eventos durante el arrastre y durante la restauración.

### Prueba manual del escenario principal

1. Conecta un monitor horizontal y otro vertical (por ejemplo, 4K al 100 % y vertical
   al 150 %).
2. Ejecuta `cargo run -- --show` y comprueba que el panel indica ambos monitores.
3. Abre Chrome en el monitor horizontal, redimensiónalo y arrástralo al vertical:
   debe adaptarse una sola vez al soltar.
4. Devuélvelo al monitor horizontal: debe recuperar tamaño y posición originales.
5. Revisa `%LOCALAPPDATA%\WindowMemory\geometries.toml`: habrá una entrada `chrome`
   por cada monitor y el log registrará una única restauración por movimiento.

## Integración continua y releases

`.github/workflows/ci.yml` comprueba formato, clippy y pruebas, y en cada etiqueta `v*`
compila `window-memory.exe`, genera `SHA256SUMS.txt` y publica el release con ambos
archivos.

## Limitaciones de esta versión

- Las ventanas maximizadas y minimizadas no se siguen todavía, aunque el campo
  `maximized` ya se persiste.
- No hay editor visual de aplicaciones: la lista se edita en `config.toml`.
- Un cambio de resolución de un monitor conserva sus geometrías; se encajan en el área
  de trabajo al restaurarse, pero no se recalculan en ese momento.
- Las ventanas de procesos con más privilegios pueden rechazar el reposicionamiento; el
  error queda en el log.

## Documentación

Las decisiones de diseño están en
[`docs/decisiones-window-memory.md`](docs/decisiones-window-memory.md). Los documentos
con los que se construyó la plantilla original están en
[`docs/historia/`](docs/historia) y no forman parte de la documentación de uso.

## Licencia

MPL-2.0. Consulta [LICENSE](LICENSE).
