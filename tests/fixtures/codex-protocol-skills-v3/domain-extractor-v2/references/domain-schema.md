# Domain glossary schema

Emit one JSON object with a `terms` array. Preserve the first appearance order
from the notes. Each term has `id`, `label`, `definition`, and `relations`.
Use lowercase kebab-case ids. A relation has `type` and `target`; the only
allowed relation type in this package is `emits-record`.

The `emits-record` relation means that the source term explicitly sends an
audit record to the target term. Do not create a relation from a statement
that only says the target receives a record unless the source send is also
explicitly stated.
