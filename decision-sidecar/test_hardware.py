"""Unit tests for decision-sidecar hardware detection."""

import unittest
from unittest.mock import patch, MagicMock
import sys
import os

sys.path.insert(0, os.path.dirname(__file__))
import hardware


class TestHardwareDetection(unittest.TestCase):
    def test_default_detection_produces_valid_structure(self):
        result = hardware.detect_hardware()
        self.assertIn("tier", result)
        self.assertIn("device", result)
        self.assertIn("device_name", result)
        self.assertIn("signature", result)
        self.assertIn("reason", result)
        self.assertIn("details", result)
        self.assertEqual(len(result["signature"]), 16)

    def test_cuda_detection_when_available(self):
        mock_torch = MagicMock()
        mock_torch.__version__ = "2.3.0"
        mock_torch.cuda.is_available.return_value = True
        mock_torch.cuda.get_device_name.return_value = "NVIDIA GeForce RTX 4090"
        mock_torch.cuda.get_device_capability.return_value = (8, 9)
        mock_torch.cuda.device_count.return_value = 1
        mock_torch.version.cuda = "12.1"

        with patch.dict("sys.modules", {"torch": mock_torch}):
            res = hardware.detect_hardware()
            self.assertEqual(res["tier"], "cuda")
            self.assertEqual(res["device"], "cuda")
            self.assertEqual(res["device_name"], "NVIDIA GeForce RTX 4090")
            self.assertIn("Confirmed NVIDIA GPU", res["reason"])

    def test_apple_silicon_detection_with_mps(self):
        mock_torch = MagicMock()
        mock_torch.__version__ = "2.3.0"
        mock_torch.cuda.is_available.return_value = False
        mock_torch.backends.mps.is_available.return_value = True
        mock_torch.zeros.return_value = MagicMock()

        # Simulate macOS arm64 without laya_mlx
        with patch("sys.platform", "darwin"), \
             patch("platform.machine", return_value="arm64"), \
             patch("platform.processor", return_value="Apple M3 Max"), \
             patch.dict("sys.modules", {"torch": mock_torch, "laya_mlx": None}):
            res = hardware.detect_hardware()
            self.assertEqual(res["tier"], "apple_silicon_mps")
            self.assertEqual(res["device"], "mps")
            self.assertIn("Apple Silicon (MPS", res["device_name"])

    def test_apple_silicon_fallback_to_cpu_when_mps_fails(self):
        mock_torch = MagicMock()
        mock_torch.__version__ = "2.3.0"
        mock_torch.cuda.is_available.return_value = False
        mock_torch.backends.mps.is_available.return_value = True
        mock_torch.zeros.side_effect = RuntimeError("MPS allocation failed")

        with patch("sys.platform", "darwin"), \
             patch("platform.machine", return_value="arm64"), \
             patch("platform.processor", return_value="Apple M1"), \
             patch.dict("sys.modules", {"torch": mock_torch, "laya_mlx": None}):
            res = hardware.detect_hardware()
            self.assertEqual(res["tier"], "cpu")
            self.assertEqual(res["device"], "cpu")
            self.assertIn("falling back to CPU", res["reason"])


if __name__ == "__main__":
    unittest.main()
