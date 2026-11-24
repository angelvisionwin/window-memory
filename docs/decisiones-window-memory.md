# Decisiones técnicas de Window Memory

Este documento recoge el porqué de las decisiones que no son evidentes al leer el
código. La estructura general (biblioteca, CLI, `App`, capa `platform`) es la heredada
de la plantilla de escritorio y no se ha alterado.

## 1. Detección por eventos, no por sondeo

Se usa `SetWinEventHook` con `WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS` y un
hook por evento, porque cada hook solo admite un rango y suscribirse a rangos amplios
(por ejemplo `EVENT_OBJECT_*`) multiplica el tráfico sin aportar información útil:

| Evento | Para qué |
| --- | --- |
| `EVENT_SYSTEM_MOVESIZESTART` | Fija la geometría y el monitor de partida. |
| `EVENT_SYSTEM_MOVESIZEEND` | Punto único donde se decide guardar o restaurar. |
| `EVENT_OBJECT_LOCATIONCHANGE` | Detecta recolocaciones ajenas al arrastre. |
| `EVENT_OBJECT_DESTROY` / `EVENT_OBJECT_HIDE` | Libera el estado de la ventana. |

El callback de Win32 solo comprueba que el objeto sea una ventana completa y publica
`WM_APP+3` en la ventana oculta de la bandeja (`wparam` = evento, `lparam` = `HWND`).
No abre procesos, no toca el disco y no bloquea al hilo que generó el evento; todo el
trabajo ocurre en el hilo de la interfaz, que ya tiene un bucle de mensajes.

Se ignoran los eventos de objetos que no sean `OBJID_WINDOW` / `CHILDID_SELF`: sin esa
condición llegarían también los cambios de posición de cada control de cada ventana.

## 2. Restaurar solo al finalizar el movimiento

Durante el arrastre no se hace nada: `MOVESIZESTART` solo registra la geometría de
partida (`rcWork` del monitor de origen) y `LOCATIONCHANGE` se descarta mientras la
ventana está marcada como «en movimiento». Restaurar en mitad del arrastre pelearía con
el usuario y generaría una cascada de eventos.

Al recibir `MOVESIZEEND`:

1. Se descarta la ventana si ya no es supervisable (por ejemplo, acaba maximizada).
2. Si el monitor de destino es el mismo, se guarda la geometría final sin mover nada.
3. Si cambia:
   - se guarda la geometría que la ventana tenía en el monitor de origen;
   - si hay geometría recordada para el destino, se restaura;
   - si no la hay, se calcula una adaptada (sección 4) y se guarda.
4. La geometría aplicada pasa a ser la «actual» de la ventana.

La geometría de origen se toma del estado guardado en `MOVESIZESTART`, no del
rectángulo final: ese rectángulo ya está en el monitor de destino y usarlo cambiaría el
tamaño recordado en cada viaje de ida y vuelta.

## 3. Tres barreras contra los bucles de eventos

Una restauración provoca `EVENT_OBJECT_LOCATIONCHANGE` (y puede provocar otro
`MOVESIZEEND` si el escritorio ajusta la ventana). Para que eso no se reinterprete como
una acción del usuario:

1. La geometría aplicada se registra como la actual de la ventana: si se repite el
   evento, el destino ya coincide y no hay nada que hacer.
2. `LOCATIONCHANGE` nunca restaura: solo actualiza el monitor conocido. Así, una
   aplicación que se recoloca sola no entra en conflicto con Window Memory.
3. Tras una restauración, la ventana queda en silencio 400 ms (`QUIET_AFTER_RESTORE`)
   durante los cuales se descartan sus eventos.

## 4. Geometrías relativas al área de trabajo y adaptación entre monitores

Las coordenadas se guardan como desplazamiento respecto a `rcWork`, no respecto al
escritorio virtual. Así no se invalidan al mover la barra de tareas, al cambiar el
origen del escritorio (monitores a la izquierda) ni al cambiar la resolución de otro
monitor.

Adaptación a un monitor sin geometría recordada:

- tamaño × `dpi_destino / dpi_origen` (mismo tamaño aparente, que es lo importante al
  pasar de un 4K al 100 % a un vertical al 150 %);
- el centro se conserva en proporción al área de trabajo (`center_x`, `center_y`);
- el resultado se encaja en el área de trabajo del destino.

Al restaurar una geometría guardada también se encaja, porque el área de trabajo puede
haber cambiado desde que se guardó.

La adaptación no se aplica al monitor de origen: si la ventana vuelve, se restaura la
geometría guardada tal cual (encajada), sin reescalarla, para que el viaje de ida y
vuelta sea exacto.

## 5. Identidad de monitores

`HMONITOR` solo es válido dentro de una sesión y no se puede persistir. La clave de
persistencia se construye, por orden de preferencia:

1. Nombre de dispositivo normalizado: `\\.\DISPLAY1` → `display1`. Sobrevive a cambios
   de resolución y a reinicios, y no cambia cuando el usuario reordena los monitores.
2. Si Windows no expone el nombre (sesiones remotas antiguas), la geometría del monitor:
   `rect:0,0,3840,2160`. Es una aproximación: un cambio de resolución de ese monitor
   pierde la asociación, y así se documenta.

Los DPI y el `rcWork` se refrescan al enumerar, no se persisten.

## 6. Identidad de las aplicaciones y varias instancias

La identidad es el nombre del ejecutable sin extensión y en minúsculas
(`chrome.exe` → `chrome`). Ventajas: es estable entre ejecuciones, se entiende en el
archivo de configuración (`[applications.chrome]`) y agrupa las ventanas de varias
instancias de la misma aplicación, que es el comportamiento esperado (dos ventanas de
Chrome en el mismo monitor acaban con la misma geometría).

La clase de ventana se usa para descartar cuadros de diálogo (`#32770`) y no forma parte
de la clave: incluirla fragmentaría la memoria por tipo de ventana de una misma
aplicación sin ganancia práctica en esta versión.

El nombre del ejecutable se resuelve con `OpenProcess` +
`QueryFullProcessImageNameW` y se cachea por PID (máximo 256 entradas). Solo se consulta
después de los filtros baratos, para no abrir procesos en cada `LOCATIONCHANGE`.

## 7. Filtros de elegibilidad

Se descartan, en este orden: proceso propio o desconocido, ventana invisible, hija (su
padre no es el escritorio), con propietario, de herramienta (`WS_EX_TOOLWINDOW`),
oculta por DWM (`DWMWA_CLOAKED`, ventanas fantasma de UWP), cuadros de diálogo,
minimizadas, maximizadas, con geometría menor de 80 × 80 y aplicaciones no supervisadas.

Las maximizadas y minimizadas quedan fuera de esta versión: en Windows una ventana
maximizada conserva su geometría normal en `WINDOWPLACEMENT.rcNormalPosition`, y
mezclar ambos estados exige más casos de prueba de los que conviene asumir en la
primera entrega. El campo `maximized` ya existe en el archivo de geometrías para no
romper el formato al añadirlo.

`GetWindowPlacement` se usa para leer el estado (maximizada/minimizada) porque es más
fiable que combinar `IsZoomed`/`IsIconic` en ventanas que se están restaurando; se
mantiene además la comprobación con esas funciones como respaldo.

## 8. DPI

El proceso se declara «per-monitor aware v2» antes de crear ventanas, así que
`GetWindowRect` y `SetWindowPos` trabajan en píxeles físicos y cada monitor tiene su
propio DPI (`GetDpiForMonitor` con `MDT_EFFECTIVE_DPI`).

Consecuencia práctica: una ventana que el usuario deja en un monitor de 144 DPI se
guarda con su tamaño físico; al adaptarla a un monitor de 96 DPI se reduce por 1,5 y al
volver se restaura el tamaño físico original.

## 9. Dos archivos TOML

- `config.toml`: lo escribe el usuario y lo reescribe la interfaz cuando se cambia algo.
- `geometries.toml`: lo genera la aplicación en cada restauración.

Se separan para que un archivo que se reescribe con frecuencia (cada movimiento entre
monitores) no destruya los comentarios y el orden de la configuración escrita a mano, y
para poder borrar la memoria de geometrías sin tocar los ajustes.

Las geometrías se guardan como array de tablas (`[[geometry]]`) y no como tablas
anidadas por aplicación: las claves de aplicación pueden contener caracteres que no son
válidos en una clave desnuda de TOML (`my app.exe`) y el array evita tener que citarlas.

Ambos archivos se reescriben completos; por eso los ajustes avanzados que no conoce la
interfaz (comentarios, campos desconocidos) deben editarse con la aplicación cerrada.

## 10. Concurrencia y ciclo de vida

Todo el estado de Window Memory vive en `App` (`RefCell<Manager>`), igual que el resto
del estado heredado. Los únicos hilos son los del refresco del panel, que solo reciben
una copia de los contadores (`Stats`). No hay hilos de fondo que toquen ventanas ni
archivos.

Los hooks se instalan y se retiran en el hilo de la interfaz:
`platform::window::EventHooks` es RAII (`UnhookWinEvent` en `Drop`) y `App` los suelta
al desactivar Window Memory y en `WM_DESTROY`. Desactivar la utilidad deja de recibir
eventos además de ignorarlos, que es lo que permite prometer consumo cero en reposo.

Las consultas cruzadas usan `try_borrow`/`try_borrow_mut`: los procedimientos de ventana
se reentran (menú modal, diálogo de configuración) y es preferible ignorar un evento a
entrar en pánico.

## 11. Errores de Win32

Todas las llamadas que devuelven `Result` o `BOOL` se comprueban; los fallos se anotan
en `window-memory.log` y no interrumpen el bucle de mensajes. Casos previstos:

- `SetWindowPos` rechazado por UIPI (ventana de un proceso con más privilegios).
- Ventana destruida entre el evento y su procesamiento (`WindowFacts::observe`
  devuelve `None`).
- `OpenProcess` denegado: la ventana se ignora como «ejecutable desconocido».
- Hook que no se puede instalar: se anota cuántos de los cinco quedaron activos.

## 12. Pruebas

La lógica de decisión es pura y se prueba sin crear ventanas:

- `window_memory`: conversiones relativa/absoluta, encaje, adaptación por DPI,
  adaptación de posición, archivo TOML, entradas corruptas o incompletas.
- `window_manager`: arrastre entre monitores con y sin memoria, vuelta al monitor de
  origen, redimensionado en el mismo monitor, eventos durante el arrastre, periodo de
  silencio, maximizadas, varias instancias, DPI distintos, restauración desactivada,
  utilidad desactivada, exclusiones, filtros, cambios de monitores y destrucción.
- `config`: valores por defecto, acotado del intervalo, lista de supervisión,
  exclusiones, `[applications.*]`, normalización de claves y TOML generado.

Validación manual del escenario principal en un equipo con dos monitores (4K al 150 %
en `DISPLAY2` y vertical al 100 % en `DISPLAY1`), arrastrando el Bloc de notas:

| Paso | Resultado |
| --- | --- |
| Ventana de 700 × 900 en el vertical, en (100, 100) | Guardado como `display1`. |
| Arrastre al 4K | Restaurada a 1050 × 1350 en (1075, 0): tamaño × 1,5 y centro proporcional. |
| Vuelta al vertical | Restaurada exactamente a (3940, 210) con 700 × 900. |

Una sola restauración por movimiento en el registro, sin eventos en cadena.

## 13. Limitaciones y siguientes pasos

- Seguir ventanas maximizadas usando `rcNormalPosition` y restaurar el estado
  maximizado al volver a un monitor.
- Recalcular geometrías al cambiar la resolución de un monitor (`WM_DISPLAYCHANGE` ya
  se recibe; ahora solo se reenumera y se descarta el seguimiento de monitores
  retirados).
- Editor visual de aplicaciones supervisadas; hoy la lista se edita en `config.toml`.
- Memoria por título de ventana, si se necesita para aplicaciones que albergan ventanas
  de documentos distintos (por ejemplo, un IDE con varios proyectos abiertos).
