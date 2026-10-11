# A Meridian operations plugin

A Meridian operations plugin, made by `meridian plugin new --role
operations`: it keeps the book -- opening balances, reconciliation and
breaks -- for the accounts in its scope.

It follows the operations role's common part (`meridian.roles.operations`),
so a person who knows the role knows this plugin: the same first-use path --
choose its settings, have access and a street source, confirm opening
balances, reconcile, work the breaks -- and the same pages, the daily page
first:

- **Breaks** (Open and View): how many are open and overdue, and each break;
  `read_breaks`.
- **Reconciliations** (Open and View): when the last one ran;
  `read_reconciliations`.
- **Opening balances** (Open and View): the accounts awaiting one, confirmed
  with "Confirm"; `read_opening_balances` and `confirm_opening_balance`,
  which names its check tool, `check_opening_draft`, so an agent may prepare
  it for a person to confirm (contract v20); the page opened from a prepared
  change starts from it, "prepared by" its preparer.

Add your plugin's nuance only at the role templates' blocks
(`vendor_notices`, `vendor_rows`, `vendor_columns`, `vendor_sections`).

    make ci-local     # what the plugin's CI runs
