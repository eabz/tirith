"""Plain-text invoices."""

from tally.models import Order
from tally.money import format_money
from tally.pricing import line_total, order_total


def invoice_number(year: int, sequence: int) -> str:
    """``invoice_number(2026, 42) == "INV-2026-000042"``."""
    if sequence < 1:
        raise ValueError("sequence starts at 1")
    return "INV-%04d-%06d" % (year, sequence)


def render_invoice(order: Order, number: str) -> str:
    """The invoice text, one line per field, ending with a newline."""
    totals = order_total(order)
    money = lambda amount: format_money(amount, order.currency)  # noqa: E731
    lines = ["Invoice %s" % number, "Region: %s" % order.region]
    for item in order.items:
        lines.append("  %s x%d  %s" % (item.sku, item.quantity, money(line_total(item, order.currency))))
    lines.append("Subtotal: %s" % money(totals.subtotal))
    if totals.discount:
        lines.append("Discount: -%s" % money(totals.discount))
    lines.append("Shipping: %s" % money(totals.shipping))
    lines.append("Tax: %s" % money(totals.tax))
    lines.append("Total: %s" % money(totals.total))
    return "\n".join(lines) + "\n"
