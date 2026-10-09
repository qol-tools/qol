<div align="center">

# QoL Plugin Registry

The publisher that pushes plugin releases to an OCI registry and builds the signed plugin index.

</div>

## Quick start

```bash
cargo run -p qol-plugin-registry
```

With no command it prints usage. Release CI runs `push` per plugin release and `index` to rebuild the index; the index is signed separately with minisign.

## License

PolyForm Noncommercial 1.0.0
