"""Hidden cross-task tests: every task's change must survive together.

The orders combine all features at once, so an edit lost to a concurrent
write in a shared file fails here even when its own task's module passed
at the moment the task was marked done. Scored separately.
"""

import unittest
from decimal import Decimal

from tally.invoice import render_invoice
from tally.models import Item, Order
from tally.pricing import order_total


def fields(totals):
    return (totals.subtotal, totals.discount, totals.shipping, totals.tax, totals.gift_wrap_fee,
            totals.small_order_fee, totals.total)


class IntegrationTest(unittest.TestCase):
    def everything_usd(self):
        return Order(items=[Item("BK", Decimal("12.50"), 4, Decimal("0.5"), "books"),
                            Item("GN", Decimal("9.00"), 8, Decimal("0.25"))],
                     region="US-CA", discount_code="welcome10", gift_wrap=True, notes="for  Ana")

    def test_usd_all_features(self):
        self.assertEqual(fields(order_total(self.everything_usd())),
                         (Decimal("122.00"), Decimal("18.30"), Decimal("0"), Decimal("4.44"), Decimal("5.00"),
                          Decimal("0"), Decimal("113.14")))

    def test_jpy_small_food_order(self):
        order = Order(items=[Item("TEA", Decimal("666.6"), 3, Decimal("0.2"), "food")], region="JP-13",
                      currency="JPY", discount_code="SPRING15")
        totals = order_total(order)
        self.assertEqual(fields(totals), (Decimal("2000"), Decimal("300"), Decimal("920"), Decimal("0"),
                                          Decimal("0"), Decimal("300"), Decimal("2920")))
        self.assertEqual(totals.total.as_tuple().exponent, 0)

    def test_eur_france_free_shipping(self):
        order = Order(items=[Item("W", Decimal("30.00"), 5, Decimal("1"))], region="EU-FR", currency="EUR")
        self.assertEqual(fields(order_total(order)), (Decimal("150.00"), Decimal("0"), Decimal("0"),
                                                      Decimal("30.00"), Decimal("0"), Decimal("0"),
                                                      Decimal("180.00")))

    def test_invoice(self):
        text = render_invoice(self.everything_usd(), "INV-2026-000100")
        self.assertTrue(text.startswith("Invoice INV-2026-000100\nRegion: US-CA\nNotes: for Ana\n"))
        self.assertIn("Total: 113.14 USD\n", text)
