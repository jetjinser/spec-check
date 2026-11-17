# Mutation schemas

- `mutation.manifest.v1.schema.json` — input manifest.
- `mutation.report.v1.schema.json` — `mutation run` report.
- `mutation.triage.v1.schema.json` — `mutation triage` output.

Manifests are validated at runtime. Reports and triage output are emitted with
the same `schema_version` and can be checked by any JSON Schema 2020-12
validator, for example:

```sh
python3 -m jsonschema -i mutation-out/report.json schemas/mutation.report.v1.schema.json
```

The tool does not bundle a schema validator; the schemas are the external
contract.
