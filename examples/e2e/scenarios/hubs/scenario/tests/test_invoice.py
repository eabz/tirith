import unittest
from decimal import Decimal

from tally.invoice import invoice_number, render_invoice
from tally.models import Item, Order


class InvoiceTest(unittest.TestCase):
    def test_number(self):
        self.assertEqual(invoice_number(2026, 42), "INV-2026-000042")
        with self.assertRaises(ValueError):
            invoice_number(2026, 0)

    def test_render(self):
        order = Order(items=[Item("A-1", Decimal("10.00"), 3)], region="US-NY")
        self.assertEqual(
            render_invoice(order, "INV-2026-000001"),
            "Invoice INV-2026-000001\n"
            "Region: US-NY\n"
            "  A-1 x3  30.00 USD\n"
            "Subtotal: 30.00 USD\n"
            "Shipping: 5.00 USD\n"
            "Tax: 1.20 USD\n"
            "Total: 36.20 USD\n",
        )
