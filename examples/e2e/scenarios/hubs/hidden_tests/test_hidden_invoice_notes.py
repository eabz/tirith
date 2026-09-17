"""Hidden acceptance: invoice-notes."""

import unittest
from decimal import Decimal

from tally.invoice import render_invoice
from tally.models import Item, Order


def order(notes=None):
    kwargs = {} if notes is None else {"notes": notes}
    return Order(items=[Item("A-1", Decimal("10.00"), 3)], region="US-NY", **kwargs)


class InvoiceNotesTest(unittest.TestCase):
    def test_default(self):
        self.assertEqual(Order().notes, "")

    def test_notes_line_after_region(self):
        lines = render_invoice(order("Leave at the door"), "INV-2026-000001").splitlines()
        self.assertEqual(lines[:3], ["Invoice INV-2026-000001", "Region: US-NY", "Notes: Leave at the door"])

    def test_whitespace_collapsed(self):
        lines = render_invoice(order("  ring\n twice  "), "INV-2026-000001").splitlines()
        self.assertEqual(lines[2], "Notes: ring twice")

    def test_no_line_without_notes(self):
        for notes in ("", "   \n "):
            with self.subTest(notes=notes):
                self.assertEqual(render_invoice(order(notes), "INV-2026-000001"),
                                 "Invoice INV-2026-000001\nRegion: US-NY\n  A-1 x3  30.00 USD\n"
                                 "Subtotal: 30.00 USD\nShipping: 5.00 USD\nTax: 1.20 USD\nTotal: 36.20 USD\n")
