"""Order pricing: line totals, discount, shipping, tax, and the total.

``order_total`` is the one entry point callers use; the helpers are
public so tests and the invoice can reuse them.
"""

from decimal import Decimal

from tally import rules
from tally.models import Item, Order, Totals
from tally.money import round_money


def line_total(item: Item, currency: str) -> Decimal:
    """Unit price times quantity, rounded."""
    return round_money(item.unit_price * item.quantity, currency)


def subtotal(order: Order) -> Decimal:
    """Sum of the rounded line totals."""
    return sum((line_total(item, order.currency) for item in order.items), Decimal("0"))


def apply_discount(order: Order, amount: Decimal) -> Decimal:
    """The discount on ``amount`` (the subtotal) as a positive amount."""
    rate = rules.DISCOUNT_CODES.get((order.discount_code or "").upper(), Decimal("0"))
    if sum(item.quantity for item in order.items) >= 10:
        rate += Decimal("0.05")
    rate = min(rate, Decimal("0.25"))
    return round_money(amount * rate, order.currency)


def shipping_cost(order: Order, merchandise: Decimal) -> Decimal:
    """Base fee plus weight fee for the order's zone.

    ``merchandise`` is the subtotal after discount.
    """
    zone = rules.zone_for(order.region)
    base, per_kg = rules.SHIPPING_RATES[zone]
    weight = sum((item.weight_kg * item.quantity for item in order.items), Decimal("0"))
    threshold = rules.FREE_SHIPPING_OVER.get(zone)
    if threshold is not None and merchandise >= threshold and weight <= 30:
        return round_money(Decimal("0"), order.currency)
    return round_money(base + per_kg * weight, order.currency)


def tax_for(order: Order, merchandise: Decimal) -> Decimal:
    """Sales tax on ``merchandise`` (the subtotal after discount)."""
    rate = rules.TAX_RATES.get(order.region, Decimal("0"))
    sub = subtotal(order)
    if not sub:
        return round_money(Decimal("0"), order.currency)
    taxable_lines = sum(
        (line_total(item, order.currency) for item in order.items if item.category not in rules.TAX_EXEMPT_CATEGORIES),
        Decimal("0"),
    )
    return round_money(merchandise * taxable_lines / sub * rate, order.currency)


def order_total(order: Order) -> Totals:
    """Price the whole order."""
    sub = subtotal(order)
    discount = apply_discount(order, sub)
    merchandise = sub - discount
    shipping = shipping_cost(order, merchandise)
    tax = tax_for(order, merchandise)
    gift_wrap_fee = rules.GIFT_WRAP_FEE[order.currency] if order.gift_wrap else Decimal("0")
    small_order_fee = Decimal("0")
    threshold, fee = rules.SMALL_ORDER_FEE[order.currency]
    if order.items and merchandise < threshold:
        small_order_fee = fee
    return Totals(
        subtotal=sub,
        discount=discount,
        shipping=shipping,
        tax=tax,
        total=merchandise + shipping + tax + gift_wrap_fee + small_order_fee,
        gift_wrap_fee=gift_wrap_fee,
        small_order_fee=small_order_fee,
    )
