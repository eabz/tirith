"""Hidden acceptance: currency-precision (BREAKING round_money signature)."""

import unittest
from decimal import Decimal

from tally.invoice import render_invoice
from tally.models import Item, Order
from tally.money import format_money, round_money
from tally.pricing import order_total


def jpy_order():
    return Order(items=[Item("T-1", Decimal("1234"), 3, Decimal("0.333"))], region="JP-13", currency="JPY",
                 discount_code="WELCOME10")


class CurrencyPrecisionTest(unittest.TestCase):
    def test_round_money_usd(self):
        self.assertEqual(round_money(Decimal("1.005"), "USD"), Decimal("1.01"))
        self.assertEqual(str(round_money(Decimal("2"), "EUR")), "2.00")

    def test_round_money_jpy(self):
        self.assertEqual(str(round_money(Decimal("1234.5"), "JPY")), "1235")

    def test_currency_is_required(self):
        with self.assertRaises(TypeError):
            round_money(Decimal("1"))  # pylint: disable=no-value-for-parameter

    def test_unknown_currency(self):
        with self.assertRaises(ValueError):
            round_money(Decimal("1"), "XYZ")

    def test_format_money_uses_currency_decimals(self):
        self.assertEqual(format_money(Decimal("1234.5"), "JPY"), "1235 JPY")
        self.assertEqual(format_money(Decimal("3"), "USD"), "3.00 USD")

    def test_jpy_order_is_whole_yen(self):
        totals = order_total(jpy_order())
        self.assertEqual((totals.subtotal, totals.discount, totals.shipping, totals.tax, totals.total),
                         (Decimal("3702"), Decimal("370"), Decimal("1000"), Decimal("333"), Decimal("4665")))
        for name in ("subtotal", "discount", "shipping", "tax", "total"):
            self.assertEqual(getattr(totals, name).as_tuple().exponent, 0, name)

    def test_usd_order_unchanged(self):
        order = Order(items=[Item("A-1", Decimal("19.99"), 2, Decimal("0.5"))], discount_code="WELCOME10")
        totals = order_total(order)
        self.assertEqual((totals.discount, totals.shipping, totals.tax, totals.total),
                         (Decimal("4.00"), Decimal("6.50"), Decimal("2.61"), Decimal("45.09")))

    def test_invoice_in_yen(self):
        text = render_invoice(jpy_order(), "INV-2026-000007")
        self.assertIn("  T-1 x3  3702 JPY\n", text)
        self.assertIn("Total: 4665 JPY\n", text)
