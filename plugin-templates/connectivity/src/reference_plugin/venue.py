"""A stand-in for the venue this plugin places at: replace it with your own.

A real connectivity plugin reaches its broker's API, a FIX line, or anything
else, with whichever library suits it: the SDK carries no client of any
venue. What it hands the SDK's Trader is this adapter (`Venue`): the venue's
accounts and catalogue, its orders read into the SDK's `VenueOrder`, a send
(`place`) answering the venue's order or raising `VenueRefused` (the venue
said no) or `VenueUnanswered` (it may have reached the venue), cancels and
replaces, and each exchange kept as a raw record (`keep`) before anything is
read from it.

This one is a broker held in memory, filling each order a little on every
read: it reaches nothing, so the plugin runs anywhere, and its tests use it.
"""

from __future__ import annotations

from dataclasses import replace as changed
from datetime import UTC, datetime
from decimal import Decimal
from typing import Any

from meridian.plugin.v1 import operations_pb2 as ops
from meridian.roles import connectivity
from meridian.roles.connectivity import Asked, Catalogue, Change, VenueOrder, VenueRefused

#: The venue's name as a symbol's source, and its one account's code.
SOURCE = "reference"
ACCOUNT = "REF-ACCOUNT-1"


class StandInVenue:
    """A broker held in memory, read into the Trader's values."""

    statuses: dict[str, str] = {
        "open": connectivity.WORKING,
        "partially_filled": connectivity.WORKING,
        "filled": connectivity.FILLED,
        "canceled": connectivity.CANCELLED,
        "rejected": connectivity.REJECTED,
    }
    sides: dict[int, str] = {ops.SIDE_BUY: "buy", ops.SIDE_SELL: "sell"}
    sides_said: dict[str, int] = {"buy": ops.SIDE_BUY, "sell": ops.SIDE_SELL}
    source: str = SOURCE

    def __init__(self) -> None:
        self.held: dict[str, VenueOrder] = {}
        self.kept: list[tuple[str, dict[str, Any]]] = []

    def accounts(self) -> tuple[str, ...]:
        return (ACCOUNT,)

    def catalogue(self, code: str) -> Catalogue:
        return ((ops.ORDER_TYPE_LIMIT, (ops.TIME_IN_FORCE_DAY,)),)

    def keep(self, name: str, record: dict[str, Any]) -> str:
        """A raw record kept before anything is read from it: here, in memory."""
        self.kept.append((name, record))
        return f"trading/{name}-{len(self.kept)}"

    def place(self, code: str, asked: Asked) -> tuple[VenueOrder, str]:
        order_id = str(len(self.held) + 1)
        now = datetime.now(UTC)
        order = VenueOrder(order_id, asked.placement_id, "open", symbol=asked.symbol,
                           side=self.sides.get(asked.side, ""), quantity=asked.quantity,
                           limit=asked.limit, placed_at=now, updated_at=now)  # fmt: skip
        self.held[order_id] = order
        return order, self.keep("placed", {"client_order_id": asked.placement_id})

    def orders(self, code: str) -> tuple[list[VenueOrder], str]:
        """The account's orders, each filled a little more on every read."""
        for order_id, held in list(self.held.items()):
            if held.status in ("open", "partially_filled") and held.quantity:
                filled = min(held.quantity, held.filled + 1)
                price = held.limit or Decimal(1)
                self.held[order_id] = changed(
                    held, filled=filled, average=price, last_fill=(filled - held.filled, price),
                    status="filled" if filled == held.quantity else "partially_filled",
                    filled_at=datetime.now(UTC), updated_at=datetime.now(UTC),
                )  # fmt: skip
        return list(self.held.values()), self.keep("orders", {"code": code})

    def cancel(self, code: str, order_id: str) -> str:
        held = self.held[order_id]
        if held.status == "filled":
            raise VenueRefused("not_open", "the order is filled", self.keep("refused", {}))
        self.held[order_id] = changed(held, status="canceled")
        return self.keep("cancelled", {"order_id": order_id})

    def replace(
        self, code: str, order: VenueOrder | None, order_id: str, change: Change
    ) -> tuple[VenueOrder | None, str]:
        held = changed(self.held[order_id], limit=change.limit)
        self.held[order_id] = held
        return held, self.keep("replaced", {"order_id": order_id})

    def not_replaceable(self, change: Change) -> str:
        return "the venue changes no order's quantity" if change.quantity_changed else ""
