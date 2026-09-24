---
name: less-code
description: Keep every code change as small as possible. Use whenever writing, fixing or refactoring code in this repository (features, bug fixes, UI changes, scripts), before adding new files, components, helpers, abstractions, options or dependencies.
---

# Less code

La mejor solución es la que deja el código más corto y claro posible. El diff neto ideal es cero o negativo.

## Antes de escribir

1. **Busca antes de crear.** Revisa si ya existe un componente, helper, tipo, token o patrón que resuelva el problema (grep o Serena). Reúsalo o extiéndelo en lugar de crear uno paralelo.
2. **Entiende el flujo actual.** Lee el código que vas a tocar y a quién afecta. Muchas veces el cambio correcto es mover o borrar, no añadir.
3. **Resuelve solo lo que se pidió.** Nada de opciones, flags, parámetros, variantes o casos que nadie pidió "por si acaso".

## Mientras escribes

- **Borra antes de añadir.** Si el cambio vuelve innecesario algo existente (código, props, estilos, copy, dependencias), elimínalo en el mismo cambio.
- **Sin abstracciones prematuras.** No crees una función, hook, componente o archivo nuevo para un solo uso. La regla de tres: abstrae hasta el tercer caso real.
- **Sin capas defensivas inútiles.** No agregues try/catch, validaciones, fallbacks ni tipos para situaciones que no pueden ocurrir en el flujo real.
- **Sin comentarios que repiten el código.** Comenta solo el porqué no obvio.
- **Sin dependencias nuevas** si la plataforma (Next, React, Bun, la librería que ya usamos) lo resuelve.
- **Sigue el estilo del archivo.** No reformatees ni reorganices código que no es parte del cambio.

## Al terminar

1. Revisa tu diff completo y pregúntate por cada bloque añadido: ¿se puede borrar, reusar algo existente o escribirse más corto sin perder claridad?
2. Corre `/simplify` sobre el cambio si está disponible, y luego `bun run check` (knip detecta exports y dependencias muertas).
3. Reporta el balance: líneas añadidas / borradas (`git diff --shortstat`) y qué eliminaste.
