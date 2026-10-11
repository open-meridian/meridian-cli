# A Meridian custody plugin

A Meridian custody plugin, made by `meridian plugin new --role custody`: it
brings a custodian's accounts and statements into a deployment, from a
stand-in custodian (`custodian.py`) you replace with your own.

It follows the custody role's common part (`meridian.roles.custody`), so a
person who knows the role knows this plugin: the same first-use path --
enter the credentials, connect, have accounts granted by a deployment admin,
link them, see statements -- and the same pages, sections and "not set up
yet" states:

- **Connections** (Manage): each connection, its state, the accounts it
  reaches and when it was last read; `read_connections`.
- **Account links** (Manage): the external accounts it reports beside the
  accounts the deployment admin granted this plugin, linked with "Link";
  `read_account_links` and `link_account`.
- **Statements** (Open and View): a row per account the person may read;
  `read_statements`.
- **History** (Open and View): the custodian's activity on those accounts, as
  reported; `read_account_activities`. A plugin keeping raw records adds
  **Raw records** from `meridian/roles/custody/raw_records.html`.

Its conversion (`convert.py`) puts what the custodian says into the
deployment in the contract's words, and its tests hold it to every one of
the custody suite's 42 cases (`tests/test_suite.py`), each mapped to one of
the custodian's responses. Each act names its check tool, so an agent may
prepare it: linking is checked by `check_link_account`.

Add the custodian's nuance only at the role templates' blocks
(`vendor_notices`, `vendor_rows`, `vendor_columns`, `vendor_sections`),
never by renaming, reordering or dropping a common item.

    make ci-local     # what the plugin's CI runs
