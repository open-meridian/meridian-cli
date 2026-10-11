"""The conversion: what the custodian said, put into the deployment in the
contract's words, once, at the edge.

Every number is parsed from the custodian's own text as a `Decimal`, never
through a `float`. Every code the custodian uses is converted to the
platform's closed list where it maps, and kept as reported where it does
not, never guessed. A value the custodian did not state is closed with its
provenance said (`meridian.edge.derived`, `supplied`), and a row names the raw
record it was converted from. Securities are resolved, stating what the
custodian says of them; what does not resolve is reported, never minted.

`meridian.suites.run("custody", producers)` holds this to the role's suite
(tests/test_suite.py): each case mapped to one of the custodian's responses,
run through this code, against a recorder standing in for the sidecar.
"""

from __future__ import annotations

from dataclasses import dataclass
from decimal import Decimal
from typing import Any

import meridian
from meridian.edge import as_reported, derived, supplied

#: How the custodian is named on what this plugin reports.
SOURCE = "reference"

#: The custodian's connection states, by its codes, as the platform's.
SYNC_STATES = {
    "OK": "current",
    "STALE": "stale",
    "REAUTH": "needs_sign_in",
    "DISABLED": "disabled",
    "T_PLUS_1": "delayed_by_design",
    "NO_HOLDINGS": "holdings_unavailable",
}
#: Its account types, as the platform's account kinds.
ACCOUNT_KINDS = {"CASH": "cash", "MARGIN": "margin", "IRA": "retirement", "ROTH": "retirement"}
#: Its activity types, as the platform's activity kinds.
ACTIVITY_KINDS = {
    "BUY": "purchase",
    "SELL": "sale",
    "REINVEST": "reinvestment",
    "DIV": "dividend",
    "INT": "interest",
    "FEE": "fee",
    "TAX": "tax",
    "SPLIT": "split",
    "CORP": "corporate_action",
    "XFER_IN": "transfer_in",
    "XFER_OUT": "transfer_out",
    "DEPOSIT": "contribution",
    "WITHDRAW": "withdrawal",
    "JOURNAL": "journal",
}
#: Its security kinds, as the platform's asset classes and instrument types.
ASSET_CLASSES = {
    "EQ": ("equity", None),
    "MMF": ("fund", "money_market_fund"),
    "ETF": ("fund", None),
}


def amount(text: Any) -> Decimal:
    """A number as the custodian wrote it."""
    return Decimal(str(text))


@dataclass
class Converter:
    """The plugin's conversion, against whatever operations it is given: the
    plugin's own when it runs, a recorder's in its tests."""

    plugin: Any

    # ── The connection and its accounts ─────────────────────────────────

    async def sync(self, connection: dict[str, Any]) -> None:
        """A connection's state, for each account it reaches, and how far
        back its activity can be read where the custodian says."""
        for account in connection["accounts"]:
            await self.plugin.report_sync_status(
                source=SOURCE,
                external_account_id=account,
                connection_healthy=connection["status"] in ("OK", "T_PLUS_1"),
                state=SYNC_STATES.get(connection["status"]),
                status_detail=connection.get("message", ""),
                history_from=connection.get("history_from", ""),
            )

    async def accounts(self, listed: list[dict[str, Any]]) -> None:
        """The accounts the connection reaches, each with its kind, or the
        custodian's type as reported where it maps to none."""
        await self.plugin.report_external_accounts(
            accounts=[
                meridian.ExternalAccount(
                    external_account_id=each["id"],
                    name=each["name"],
                    account_kind=ACCOUNT_KINDS.get(each["type"]),
                    account_kind_as_reported=(
                        None
                        if each["type"] in ACCOUNT_KINDS
                        else as_reported(
                            "reference:account-type", each["type"], each["type_name"]
                        )
                    ),
                )
                for each in listed
            ]
        )

    async def link_new(self, account: dict[str, Any], name: str, caller: str | None) -> Any:
        """Link an account the connection reaches to a new account of the
        deployment's, created as the person names it."""
        return await self.plugin.link_external_account(
            external_account_id=account["id"],
            new_account_name=name,
            new_account_custodian=account["institution"],
            new_account_type=ACCOUNT_KINDS.get(account["type"], "other"),
            acting_for=caller,
        )

    # ── Holdings ────────────────────────────────────────────────────────

    async def security(self, position: dict[str, Any], as_of_ns: int) -> str:
        """The deployment's instrument for a position's security, stating
        what the custodian says of it; empty, the miss reported, where it
        does not resolve to one."""
        named = [meridian.Identifier(scheme="symbol", value=position["symbol"], source=SOURCE)]
        asset_class, instrument_type = ASSET_CLASSES.get(position.get("kind", ""), (None, None))
        found = await self.plugin.resolve_identifier(
            identifiers=named,
            as_of_ns=as_of_ns,
            stated_asset_class=asset_class,
            stated_instrument_type=instrument_type,
            stated_currency=position.get("currency", ""),
            stated_description=position.get("description", ""),
        )
        if found.found and found.instrument_id:
            return str(found.instrument_id)
        await self.plugin.report_missing_instrument(
            source=SOURCE,
            identifiers=named,
            as_of_ns=as_of_ns,
            reason=found.miss_reason or "not_found",
            asset_class=asset_class,
            asset_class_as_reported=(
                None
                if asset_class
                else as_reported(
                    "reference:security-kind",
                    position.get("kind", ""),
                    position.get("kind_name"),
                )
            ),
        )
        return ""

    async def cash(self, currency: str, as_of_ns: int) -> str:
        found = await self.plugin.resolve_identifier(
            identifiers=[meridian.Identifier(scheme="iso4217", value=currency)],
            as_of_ns=as_of_ns,
        )
        return str(found.instrument_id) if found.found else ""

    async def statement(
        self, account: dict[str, Any], connection: dict[str, Any], as_of_ns: int
    ) -> None:
        """An account's holdings as of a day: its statement, then a row per
        security and its cash net of any money market fund the custodian
        counts in it. Nothing is recorded where the custodian's figures
        cannot both hold: a fund it counts in cash larger than that cash."""
        positions = list(account.get("positions", []))
        balance = account.get("cash")
        swept = [p for p in positions if p.get("counted_in_cash")]
        in_funds = sum((amount(p["units"]) for p in swept), Decimal(0))
        if balance is not None and in_funds > amount(balance["amount"]):
            return
        provenance = []
        institution = account.get("institution", "")
        if not institution:
            institution = connection["institution"]
            provenance.append(derived("institution", "the connection's institution"))
        statement = await self.plugin.record_holdings_statement(
            source=SOURCE,
            external_statement_id=f"{account['id']}:{account['as_of']}",
            external_account_id=account["id"],
            institution=institution,
            as_of_date=account["as_of"],
            expected_rows=len(positions) + (1 if balance is not None else 0),
            raw_record=self.plugin.raw_record(f"holdings/{account['id']}/{account['as_of']}"),
            provenance=provenance,
        )
        for position in positions:
            await self.holding(statement.statement_id, account, position, as_of_ns)
        if balance is not None:
            await self.plugin.record_holding(
                statement_id=statement.statement_id,
                external_account_id=account["id"],
                instrument_id=await self.cash(balance["currency"], as_of_ns),
                quantity=amount(balance["amount"]) - in_funds,
                side="long",
                raw_record=self.plugin.raw_record(
                    f"balances/{account['id']}/{account['as_of']}"
                ),
                provenance=(
                    [derived("quantity", "cash net of money market funds counted in cash")]
                    if swept
                    else []
                ),
            )

    async def holding(
        self,
        statement_id: str,
        account: dict[str, Any],
        position: dict[str, Any],
        as_of_ns: int,
    ) -> None:
        instrument = await self.security(position, as_of_ns)
        units = amount(position["units"])
        provenance = []
        settled = position.get("settled")
        if settled is None:
            settled_units = units - sum(
                (amount(p["units"]) for p in position.get("pending", [])), Decimal(0)
            )
            provenance.append(
                derived("settle_date_quantity", "quantity less what is pending settlement")
            )
        else:
            settled_units = amount(settled)
        average_cost = None
        if "average_cost" in position:
            currency = position.get("currency", "")
            if not currency:
                currency = account["base_currency"]
                provenance.append(
                    derived("average_cost.currency_code", "the account's base currency")
                )
            average_cost = meridian.Money(amount(position["average_cost"]), currency)
        await self.plugin.record_holding(
            statement_id=statement_id,
            external_account_id=account["id"],
            instrument_id=instrument,
            unresolved_identifiers=(
                []
                if instrument
                else [
                    meridian.Identifier(
                        scheme="symbol", value=position["symbol"], source=SOURCE
                    )
                ]
            ),
            quantity=units,
            settle_date_quantity=settled_units,
            side="long",
            average_cost=average_cost,
            pending=[
                meridian.ReportedPending(
                    value_date=p["value_date"], quantity=amount(p["units"])
                )
                for p in position.get("pending", [])
            ],
            raw_record=self.plugin.raw_record(
                f"holdings/{account['id']}/{account['as_of']}/{position['symbol']}"
            ),
            provenance=provenance,
        )

    # ── Activity ────────────────────────────────────────────────────────

    async def activity(
        self, account: str, item: dict[str, Any], plan_links: list[dict[str, str]]
    ) -> None:
        """One activity on an account, as the custodian lists it: its kind,
        or its type as reported; its units, price and cash where it states
        them; its security, or a plan's own fund code linked by a person in
        the plugin's settings, or kept as reported."""
        kind = ACTIVITY_KINDS.get(item["type"])
        instrument, as_reported_code, provenance = "", None, []
        if "plan_code" in item:
            linked = next((r for r in plan_links if r["code"] == item["plan_code"]), None)
            if linked is not None:
                instrument = linked["instrument"]
                provenance.append(supplied("instrument_id", linked["changed_by"]))
            else:
                as_reported_code = as_reported(
                    "reference:plan-code", item["plan_code"], item.get("plan_name")
                )
        elif "symbol" in item:
            instrument = await self.security(item, 0)

        def money(key: str) -> meridian.Money | None:
            return meridian.Money(amount(item[key]), item["currency"]) if key in item else None

        await self.plugin.record_activity(
            external_account_id=account,
            source=SOURCE,
            activity=meridian.CustodialActivity(
                external_activity_id=item["id"],
                kind=kind,
                kind_as_reported=(
                    None
                    if kind
                    else as_reported(
                        "reference:activity-type", item["type"], item.get("type_name")
                    )
                ),
                instrument_id=instrument,
                instrument_as_reported=as_reported_code,
                trade_date=item["date"],
                units=amount(item["units"]) if "units" in item else None,
                price=money("price"),
                amount=money("amount"),
                description=item.get("description", ""),
                raw_record=self.plugin.raw_record(f"activity/{account}/{item['id']}"),
                provenance=provenance,
            ),
        )

    async def plan_code_linked(
        self, account: str, item: dict[str, Any], link: dict[str, str], now_ns: int
    ) -> None:
        """A plan's own fund code a person linked after an activity naming it
        was recorded: the activity re-resolved, kept as first recorded."""
        await self.plugin.re_resolve_activity(
            external_account_id=account,
            source=SOURCE,
            external_activity_id=item["id"],
            instrument_id=link["instrument"],
            provenance=supplied("instrument_id", link["changed_by"]),
            resolved_at_ns=now_ns,
        )
