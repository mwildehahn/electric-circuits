# A table is identified by (schema, name); the canonical spelling is `schema.name` everywhere

Status: accepted (2026-08-21)

Upstream keys tables by bare name: introspection is filtered to `public`, the pgoutput `Relation`
namespace is decoded and discarded, and every SQL site quotes the name as a single identifier — so two
same-named tables in different schemas collide silently and a non-`public` table cannot be served at
all. Table identity becomes a typed `TableRef { schema, name }` inside the engine, with one canonical
serialisation `schema.name` on the wire (the `table` fields of the native API, the envelope `type`),
in the durable catalog, and in sharing signatures — always qualified, `public.users` never `users`. A
bare name is accepted only at API ingress as shorthand for `public.<name>` and canonicalised there;
`ELECTRIC_CIRCUITS_PG_TABLES` accepts `schema.name` and `schema.*`, and `*` keeps meaning "every
`public` table with a primary key" (introspecting every schema would put `REPLICA IDENTITY FULL` on
managed system schemas). The `__el_sync` sentinel is `public.__el_sync`.

## Considered options

- A canonical qualified *string* with no type: least churn, but every `quote_ident` caller must
  remember to split, and a quoted identifier containing a dot is ambiguous.
- Relation OID as identity: rejected — OIDs do not survive dump/restore or table recreation, and the
  catalog outlives them.

## Consequences

The envelope `type` and catalog records change spelling (consumers are greenfield; clients resync at
cutover). The dotted-identifier parse rule lives in exactly one place. The `/v1/shape` adapter resolves
the schema prefix instead of stripping it, which removes a wrong-rows disclosure as a side effect.
