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
| `EVENT_SYSTEM_MOVESIZESTART` | Fija la geometría y el monitor de partida del arrastre. |
| `EVENT_SYSTEM_MOVESIZEEND` | Cierra el arrastre y decide guardar o restaurar. |
| `EVENT_OBJECT_LOCATIONCHANGE` | Detecta movimientos y redimensionados sin arrastre. |
| `EVENT_OBJECT_SHOW` | Registra las ventanas que aparecen con el punto de partida. |
| `EVENT_OBJECT_DESTROY` / `EVENT_OBJECT_HIDE` | Libera el estado de la ventana. |

El callback de Win32 solo comprueba que el objeto sea una ventana completa y publica
`WM_APP+3` en la ventana oculta de la bandeja (`wparam` = evento, `lparam` = `HWND`).
No abre procesos, no toca el disco y no bloquea al hilo que generó el evento; todo el
trabajo ocurre en el hilo de la interfaz, que ya tiene un bucle de mensajes.

Se ignoran los eventos de objetos que no sean `OBJID_WINDOW` / `CHILDID_SELF`: sin esa
condición llegarían también los cambios de posición de cada control de cada ventana.

## 2. Restaurar al terminar el movimiento, por las dos vías que existen

**Arrastre con el ratón.** Durante el arrastre no se hace nada: `MOVESIZESTART` solo
registra la geometría de partida (`rcWork` del monitor de origen) y `LOCATIONCHANGE` se
descarta mientras la ventana está marcada como «en movimiento». Restaurar en mitad del
arrastre pelearía con el usuario y generaría una cascada de eventos.

**Atajos de Windows y recolocaciones.** `Win+Mayús+flecha` (mover a otro monitor),
`Win+flecha` (ajustar a la mitad) y cualquier `SetWindowPos` externo **no generan
`MOVESIZESTART`/`MOVESIZEEND`**: se comprobó midiendo el flujo de eventos, que solo
contiene `EVENT_OBJECT_LOCATIONCHANGE`. Por eso existe una segunda vía:

1. Al arrancar se enumeran una sola vez las ventanas visibles (`EnumWindows`) y se
   registra su monitor y geometría; `EVENT_OBJECT_SHOW` hace lo mismo con las ventanas
   que aparecen después. Sin ese punto de partida no se puede saber de dónde viene un
   movimiento que no tiene eventos de arrastre.
2. Un `LOCATIONCHANGE` que cambia de monitor no restaura en el acto: marca la ventana
   como pendiente y arma un temporizador de asentamiento (`SETTLE_DELAY`, 350 ms) que
   cada evento reinicia.
3. Cuando la ventana está quieta, se vuelve a observar y se aplica la misma decisión que
   al soltar el ratón: guardar la geometría del origen y restaurar o adaptar la del
   destino. Un cambio de tamaño en el mismo monitor también se recuerda por esta vía.

En ambos casos:

1. Se descarta la ventana si ya no es supervisable (por ejemplo, acaba maximizada).
2. Si el monitor de destino es el mismo, se guarda la geometría final sin mover nada.
3. Si cambia:
   - se guarda la geometría que la ventana tenía en el monitor de origen;
   - si hay geometría recordada para el destino, se restaura;
   - si no la hay, se calcula una adaptada (sección 4) y se guarda.
4. La geometría aplicada pasa a ser la «actual» de la ventana.

La geometría de origen se toma del estado guardado antes del movimiento, no del
rectángulo final: ese rectángulo ya está en el monitor de destino y usarlo cambiaría el
tamaño recordado en cada viaje de ida y vuelta. Si el usuario agarra la ventana mientras
hay un asentamiento pendiente, el arrastre manda: `MOVESIZESTART` cancela la espera.

Contrapartida asumida: un sistema de eventos no distingue «el usuario pulsó un atajo» de
«la aplicación se recolocó sola». Ambos acaban en las mismas reglas; si una aplicación
concreta se resiste, se excluye con `excluded` o se desactiva `auto_restore`.

## 3. Barreras contra los bucles de eventos

Una restauración provoca `EVENT_OBJECT_LOCATIONCHANGE` (y puede provocar otro
`MOVESIZEEND` si el escritorio ajusta la ventana). Para que eso no se reinterprete como
una acción del usuario:

1. La geometría aplicada se registra como la actual de la ventana: si llega el evento
   propio, el monitor y la geometría ya coinciden y no se arma ningún asentamiento.
2. El asentamiento espera 350 ms desde el último evento: una animación o una secuencia
   de varios `LOCATIONCHANGE` produce una sola decisión.
3. Tras una restauración, la ventana queda en silencio 150 ms
   (`QUIET_AFTER_RESTORE`) para descartar los eventos propios que aún estén en cola. El
   margen es corto a propósito: así un segundo atajo inmediato del usuario no se pierde.
4. Un arrastre del usuario cancela el asentamiento pendiente, de modo que nunca se
   mezclan las dos vías.

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
- Hook que no se puede instalar: se anota cuántos de los seis quedaron activos.

## 12. Pruebas

La lógica de decisión es pura y se prueba sin crear ventanas:

- `window_memory`: conversiones relativa/absoluta, encaje, adaptación por DPI,
  adaptación de posición, archivo TOML, entradas corruptas o incompletas.
- `window_manager`: arrastre entre monitores con y sin memoria, vuelta al monitor de
  origen, redimensionado en el mismo monitor, eventos durante el arrastre, periodo de
  silencio, maximizadas, varias instancias, DPI distintos, restauración desactivada,
  utilidad desactivada, exclusiones, filtros, cambios de monitores y destrucción.
- Movimientos sin arrastre (atajos de Windows): siembra del seguimiento, espera de
  asentamiento, restauración al cambiar de monitor, vuelta al origen, ajuste en el mismo
  monitor, cancelación por un arrastre posterior y olvido si la ventana desaparece.
- `config`: valores por defecto, acotado del intervalo, lista de supervisión,
  exclusiones, `[applications.*]`, normalización de claves y TOML generado.

Validación manual del escenario principal en un equipo con dos monitores (4K al 150 %
en `DISPLAY2` y vertical al 100 % en `DISPLAY1`), con el Bloc de notas:

| Paso | Resultado |
| --- | --- |
| Ventana de 700 × 900 en el vertical, en (100, 100) | Guardado como `display1`. |
| Arrastre al 4K | Restaurada a 1050 × 1350 en (1075, 0): tamaño × 1,5 y centro proporcional. |
| Vuelta al vertical | Restaurada exactamente a (3940, 210) con 700 × 900. |
| `SetWindowPos` al 4K (mismo flujo que `Win+Mayús+flecha`) | Restaurada a 1050 × 1350 tras 350 ms de asentamiento. |
| Dos movimientos seguidos separados 600 ms | Una restauración por movimiento, sin perder el segundo. |
| `Win+flecha` real sobre otra ventana | Geometría anotada en el registro (`recordada en display2`), sin sondeos. |

Una sola restauración por movimiento en el registro, sin eventos en cadena.

## 13. Limitaciones y siguientes pasos

- Un sistema de eventos no distingue un movimiento pedido por el usuario (atajo de
  Windows) de uno iniciado por la propia aplicación. Las dos vías comparten reglas; si
  una aplicación se resiste, se excluye o se desactiva `auto_restore`.
- Seguir ventanas maximizadas usando `rcNormalPosition` y restaurar el estado
  maximizado al volver a un monitor.
- Recalcular geometrías al cambiar la resolución de un monitor (`WM_DISPLAYCHANGE` ya
  se recibe; ahora solo se reenumera y se descarta el seguimiento de monitores
  retirados).
- Editor visual de aplicaciones supervisadas; hoy la lista se edita en `config.toml`.
- Memoria por título de ventana, si se necesita para aplicaciones que albergan ventanas
  de documentos distintos (por ejemplo, un IDE con varios proyectos abiertos).
