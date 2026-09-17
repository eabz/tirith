import unittest
from decimal import Decimal

from tally.money import format_money, parse_money, round_money, to_decimal


class MoneyTest(unittest.TestCase):
    def test_round_half_up(self):
        self.assertEqual(round_money(Decimal("1.005"), "USD"), Decimal("1.01"))
        self.assertEqual(round_money(Decimal("2.344"), "USD"), Decimal("2.34"))
        self.assertEqual(round_money(Decimal("1234.5"), "JPY"), Decimal("1235"))

    def test_format(self):
        self.assertEqual(format_money(Decimal("3.5"), "USD"), "3.50 USD")
        self.assertEqual(format_money(Decimal("1234.5"), "JPY"), "1235 JPY")

    def test_parse_plain(self):
        self.assertEqual(parse_money(" 12.50 "), Decimal("12.50"))

    def test_parse_symbols(self):
        self.assertEqual(parse_money("$1,234.50"), Decimal("1234.50"))
        self.assertEqual(parse_money("(12.00)"), Decimal("-12.00"))

    def test_float_refused(self):
        with self.assertRaises(TypeError):
            to_decimal(1.5)
