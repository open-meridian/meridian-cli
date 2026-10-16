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
    # The custodian's own type for the account, in its words, which the
    # account map shows beside the name.
    account_type: str = ""


@dataclass
class Custodian:
    """The custodian as this plugin reads it."""

    connection: str = "Reference custodian · Individual"
    state: str = "Connected"
    last_read: str = ""
    accounts: tuple[Account, ...] = field(
        default_factory=lambda: (
            Account(
                "REF-1001",
                "Individual brokerage",
                "Reference custodian",
                "2026-10-09",
                "Margin",
            ),
            Account("REF-1002", "Roth IRA", "Reference custodian", "2026-10-09", "Roth IRA"),
        )
    )
