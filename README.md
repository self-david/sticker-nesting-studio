# Sticker Nesting Studio

Aplicacion de escritorio para organizar stickers, calcular disposicion y generar hojas de impresion.

## Caracteristicas

- Biblioteca local de stickers con etiquetas y busqueda.
- Organizacion de copias y rotacion en hojas de impresion.
- Generacion y exportacion de hojas PDF.
- Importacion desde archivos o carpetas.
- Integracion opcional con modelos de IA.

## Tecnologias

- Tauri 2
- React 19
- TypeScript
- Vite
- Rust

## Requisitos

- Bun
- Rust y Cargo
- Dependencias de desarrollo de Tauri 2 para Windows

## Desarrollo

Instalar dependencias:

```powershell
bun install
```

Ejecutar la aplicacion en modo desarrollo:

```powershell
bun run app:dev
```

## Comandos

Compilar solo el frontend:

```powershell
bun run build
```

Previsualizar el frontend:

```powershell
bun run preview
```

Crear el instalador de escritorio:

```powershell
bun run app:build
```
