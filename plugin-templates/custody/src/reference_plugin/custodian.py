"""A stand-in custodian: one connection reaching two accounts, each with its
latest statement. A real custody plugin reads its custodian's API with the
credentials in its settings; replace this with yours, keeping what it answers
in the custody role's words."""

from __future__ import annotations

from dataclasses import dataclass, field


@dataclass(frozen=True)
class Account:
    external_account_id: str
    name: str
    detail: str
    as_of: str


@dataclass
class Custodian:
    """The custodian as this plugin reads it."""

    connection: str = "Reference custodian · Individual"
    state: str = "Connected"
    last_read: str = ""
    accounts: tuple[Account, ...] = field(
        default_factory=lambda: (
            Account(
                "REF-1001", "Individual brokerage", "Reference custodian · Margin", "2026-10-09"
            ),
            Account("REF-1002", "Roth IRA", "Reference custodian · IRA", "2026-10-09"),
        )
    )
