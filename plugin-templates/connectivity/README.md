# A Meridian connectivity plugin

A Meridian connectivity plugin, made by `meridian plugin new --role
connectivity`: it places what its ems released at a venue, in the account its
ems mapped, and reports what the venue did -- acknowledged, filled, cancelled,
rejected -- with every fill into the book.

Its whole work is the SDK's (`meridian.roles.connectivity.Trader`): a
placement sent at most once, an unknown outcome resolved from the venue's
orders, fills as stated or inferred, cancels and replaces, the firm's pauses
and stops, the catalogue, and fills of orders placed outside the platform. The
plugin keeps what is its venue's alone: its client, and an adapter reading the
venue's words into the SDK's (`venue.py`, a stand-in broker here; replace it
with your venue's). How it reaches the venue -- an API, FIX -- is its own.

- **Trading** (Manage, connectivity): the key, the environment, each account
  read-only or with trading enabled, and the placements working, held back or
  of unknown outcome; `read_trading`. "Enable trading" on a read-only account
  for the connectivity role's admin (`enable_trading`, checked by
  `check_enable_trading`).

Tests use fakes only: never a real broker, never a token.

    make ci-local     # what the plugin's CI runs
