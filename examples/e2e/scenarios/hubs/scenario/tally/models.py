"""Order data. Amounts are ``Decimal`` in the order's currency, never float."""

from dataclasses import dataclass, field
from decimal import Decimal
from typing import List, Optional


@dataclass
class Item:
    """One order line."""

    sku: str
    unit_price: Decimal
    quantity: int = 1
    weight_kg: Decimal = Decimal("0")
    category: str = "general"


@dataclass
class Order:
    """What the customer bought and where it ships."""

    items: List[Item] = field(default_factory=list)
    # "<zone>-<area>", e.g. "US-CA", "EU-DE", "JP-13". The zone picks the
    # shipping table, the full region the tax rate.
    region: str = "US-CA"
    currency: str = "USD"
    discount_code: Optional[str] = None


@dataclass
class Totals:
    """The result of ``tally.pricing.order_total``."""

    subtotal: Decimal
    discount: Decimal
    shipping: Decimal
    tax: Decimal
    total: Decimal
