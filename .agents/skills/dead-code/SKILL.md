---
name: dead-code
description: Find and remove dead code and unused files — unreferenced images, videos, fonts and PDFs in public/, superseded media versions, orphan scripts and docs, unused exports, components and dependencies. Use when the user asks to clean up, remove unused/dead code or assets, shrink the repo, or after a redesign or refactor that replaced media, pages or components.
---

# Dead code

Borra lo que ya nada usa: código, dependencias, imágenes, videos, scripts y docs. Cada archivo que se borra es uno menos que leer, mantener, desplegar y confundir con el vigente.

## Flujo

Sigue los pasos en orden. **No borres nada antes de que el usuario vea el reporte.**

### 1. Escanear

Corre los dos escáneres (solo leen, nunca editan):

```bash
bun run knip                                             # código: archivos, exports, tipos y dependencias sin uso
node .agents/skills/dead-code/scripts/scan.mjs .         # assets, scripts y docs sin referencia
node .agents/skills/dead-code/scripts/scan.mjs . --json  # lo mismo, para triage
```

`scan.mjs` cruza cada archivo de `public/` (y los assets en `src/`, `app/`, `content/`) contra el código y la config. Los clasifica en:

| Grupo | Significado | Acción por defecto |
|---|---|---|
| Sin referencia | Ni el código ni los docs lo mencionan | Candidato a borrar |
| Solo mencionados en docs | Solo aparece en docs o manifiestos de trabajo; suele ser una versión anterior de un medio ya reemplazado | Candidato a borrar tras confirmar cuál es la versión vigente |
| Posible ruta dinámica | Coincide con un prefijo armado en tiempo de ejecución (`` `/media/${slug}.webp` ``) | Revisar a mano: abrir el código que arma la ruta |
| Cargados por convención | favicon, icon, robots, sitemap, manifest, OG, `.well-known` | Conservar |
| Scripts que nada ejecuta | No están en package.json, docs ni código | Preguntar si es herramienta manual vigente |
| Docs que nadie enlaza | Ningún otro `.md` los enlaza | Enlazar desde docs/README.md o borrar si ya no aplica |

También revisa con `du -sh * .[!.]* | sort -h` si hay carpetas grandes fuera de `public/` (exportaciones, respaldos, salidas generadas) que deberían ignorarse o borrarse.

### 1b. Lo que knip no ve

Knip solo encuentra archivos, exports y dependencias sin uso. El código muerto más grande suele estar en otro lado; búscalo con estas técnicas:

- **Alcanzabilidad desde el punto de entrada (Python y scripts).** Knip no analiza Python. Para cada `tools/**/*.py` o script usado por `package.json`, parsea el archivo (`python3 -c "import ast…"`) y recorre el grafo de llamadas desde `main()`/el bloque `if __name__`: las funciones y constantes no alcanzables son muertas, **aunque se llamen entre sí** (un bloque muerto que se autorreferencia no aparece como aislado en ningún grafo).
- **Variantes de componentes.** Para cada `cva(...)`/mapa de variantes (`variant`, `size`, `tone`), cuenta usos reales con grep (`variant="x"`, `size="x"`, y props dinámicos). Borra variantes sin uso y su CSS/tokens; fusiona variantes con clases idénticas.
- **Duplicados entre archivos.** Busca funciones, constantes y tipos con el mismo nombre en varios archivos (`grep -rhoE "function \w+|const \w+ =" src | sort | uniq -c | sort -rn`) y compara sus cuerpos; knip no los ve porque cada copia es local a su archivo. Consolida en un módulo **dentro del mismo repo** (nunca entre repos).
- **Helpers mecánicos.** Funciones con sufijos numéricos (`…2`, `…3`) o con 10+ parámetros desestructurados suelen ser extracciones automáticas; inlinearlas reduce líneas y saltos.
- **Mapas de medios y contenido.** Un archivo referenciado solo desde una entrada muerta de un mapa (`media-assets.ts`, `site.ts`, diccionarios i18n) aparece como "usado" en el escáner. Verifica que cada clave del mapa se lea en alguna vista (TypeScript "find references" o grep de la clave) y borra la entrada junto con sus archivos.
- **Esquema huérfano.** Tablas o columnas del esquema de BD que ningún código lee ni escribe (revisa también funciones SQL/RLS antes de marcarlas).

**Sobre graphify:** úsalo para orientarte (god nodes, comunidades, rutas), no como detector de muertos. Sus nodos aislados dan casi solo falsos positivos: no registra usos dentro del mismo archivo, convenciones de Next (page, layout, loading, route, server actions), crons ni lecturas con `readFile`. Confirma siempre con grep, Serena o el AST.

### 2. Triage

Por cada candidato, confirma antes de proponerlo:

- **Rutas armadas:** busca el nombre sin extensión y partes del nombre (`hero-elementary`), no solo la ruta completa. Revisa mapas de medios (`media-assets.ts`, `site.ts`), `srcset`, variantes `-720`/`-poster`, CSS (`url(...)`) y `next.config` (redirects, headers).
- **Versiones reemplazadas:** si existe `foo-2026.mp4` en uso y `foo.mp4` solo en docs, la anterior es basura; los docs que la citan son historia y se actualizan o se dejan como registro.
- **Contenido servido por URL externa:** PDFs o imágenes enlazados desde correos, redes o sitios de terceros no aparecen en el código. Si el nombre parece público y estable (reglamentos, circulares), pregunta.
- **Código:** knip es la fuente de verdad; si marca algo que sí se usa (carga dinámica, convención de Next), corrige la config de knip en vez de conservar el falso positivo.
- **Marca e identidad:** logos, favicons, `branding/` y fuentes se conservan salvo que el usuario diga lo contrario.

### 3. Reportar

Antes de borrar, entrega un resumen agrupado con tamaño y fecha del último commit:

```
borrar   public/video/paseo-hero-elementary.mp4      6.0 MB  2026-09-21  reemplazado por …-topiq-2026.mp4
borrar   src/components/OldHero.tsx                  knip    2026-08-10  sin imports
revisar  public/media/niveles/k1.webp                0.4 MB  ruta dinámica niveles/${id}
conservar public/brand/logo.svg                              marca
→ 32 archivos, 71 MB a borrar; 2 a revisar.
```

Pregunta qué grupos aplicar o si procede con todo.

### 4. Borrar

- Borra con `git rm` (o `rm` si no está versionado); el historial de git es el respaldo, no dejes copias `_old`, `backup/` ni carpetas de "por si acaso".
- Quita en el mismo cambio las referencias muertas: entradas en manifiestos, docs que listan el archivo como vigente, dependencias en package.json.
- Corre `bun run check` y `bun run build`; si el repo tiene sitio público, abre las páginas afectadas y confirma que no hay 404 de medios.
- Vuelve a correr `scan.mjs` y reporta cuánto se redujo (archivos y MB).

## Reglas

- Nunca borres antes del reporte, ni fuera del repo actual.
- Ante la duda, pregunta; un asset roto en producción cuesta más que uno de sobra.
- No reescribas historial de git para quitar binarios pesados salvo que el usuario lo pida explícitamente.
