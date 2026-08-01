// ignore_for_file: dangling_library_doc_comments

// core/bcs_utils.dart
// BCS utilities for Kanari Pay.
///
/// ชุด utility functions สำหรับ BCS encoding/decoding
/// สามารถนำไปใช้ซ้ำได้ในทุกที่ที่ต้องการทำงานกับ BCS data
///
/// ## ตัวอย่างการใช้งาน
/// ```dart
/// import 'package:kanari_pay/core/core.dart';
///
/// // Normalize address
/// final addr = BcsUtils.normalizeAddress('0x123...');
///
/// // Encode amount
/// final bytes = BcsUtils.encodeU64(1000);
///
/// // Build transaction args
/// final args = TransactionArgs()
///   ..addAddress(recipient)
///   ..addAmount(amount);
/// ```
import 'package:bcs/bcs.dart';

/// BCS utilities class
class BcsUtils {
  /// Normalize address to standard format
  /// Supports both short addresses (0x2) and full addresses (0x + 64 hex chars)
  static String normalizeAnyAddress(String addr) {
    var clean = addr.startsWith('0x') ? addr.substring(2) : addr;

    if (!RegExp(r'^[0-9a-fA-F]+$').hasMatch(clean)) {
      throw ArgumentError('Invalid hexadecimal characters in address: $clean');
    }

    // Short address - keep as-is
    if (clean.length < 64) {
      return '0x${clean.toLowerCase()}';
    }

    // Full address - validate length
    if (clean.length != 64) {
      throw ArgumentError(
        'Address must be exactly 64 hex characters (32 bytes). '
        'Got ${clean.length} characters.',
      );
    }

    return '0x${clean.toLowerCase()}';
  }

  /// Normalize address to 0x followed by 64 hex characters.
  /// Short-form addresses like `0x2` are left-padded to 32 bytes.
  static String normalizeAddress(String addr) {
    var clean = addr.startsWith('0x') ? addr.substring(2) : addr;

    if (!RegExp(r'^[0-9a-fA-F]+$').hasMatch(clean)) {
      throw ArgumentError('Invalid hexadecimal characters in address: $clean');
    }

    if (clean.length > 64) {
      throw ArgumentError(
        'Address must be exactly 64 hex characters (32 bytes). '
        'Got ${clean.length} characters.',
      );
    }

    return '0x${clean.padLeft(64, '0').toLowerCase()}';
  }

  /// Normalize object ID to standard format (0x + 64 hex chars)
  static String normalizeObjectId(String objectId) {
    var clean = objectId.startsWith('0x') ? objectId.substring(2) : objectId;

    if (clean.isEmpty || !RegExp(r'^[0-9a-fA-F]+$').hasMatch(clean)) {
      throw ArgumentError('Invalid object ID format: $objectId');
    }

    clean = clean.padLeft(64, '0').toLowerCase();

    if (clean.length != 64) {
      throw ArgumentError(
        'Object ID must be 32 bytes (64 hex chars). Got ${clean.length} characters.',
      );
    }

    return '0x$clean';
  }

  /// Convert hex string to bytes (รองรับ odd-length)
  static List<int> hexToBytes(String hexStr) {
    final clean = hexStr.startsWith('0x') ? hexStr.substring(2) : hexStr;
    final padded = clean.length.isOdd ? '0$clean' : clean;

    final bytes = <int>[];
    for (int i = 0; i < padded.length; i += 2) {
      bytes.add(int.parse(padded.substring(i, i + 2), radix: 16));
    }
    return bytes;
  }

  /// Convert bytes to hex string
  static String bytesToHex(List<int> bytes) {
    return '0x${bytes.map((b) => b.toRadixString(16).padLeft(2, '0')).join()}';
  }

  /// Encode u64 to BCS format (little-endian)
  static List<int> encodeU64(int value) {
    final bcs = Bcs.u64();
    return bcs.serialize(value).toBytes();
  }

  /// Decode u64 from BCS bytes (little-endian)
  ///
  /// @throws ArgumentError ถ้า bytes ไม่ครบ 8 bytes
  static int decodeU64(List<int> bytes) {
    if (bytes.length != 8) {
      throw ArgumentError(
        'u64 must be exactly 8 bytes. Got ${bytes.length} bytes.',
      );
    }

    int value = 0;
    for (int i = 0; i < 8; i++) {
      value |= (bytes[i] << (i * 8));
    }
    return value;
  }

  /// Decode the balance field from a Coin<T> object layout: UID (32 bytes)
  /// followed by Balance<T>.value (u64 little-endian).
  static int? readCoinObjectBalance(List<int> data) {
    if (data.length < 40) return null;
    return decodeU64(data.sublist(32, 40));
  }

  /// True only for actual Coin<T> object types.
  static bool isCoinObjectType(String objectType) {
    final normalized = objectType.replaceAll(' ', '');
    return normalized.contains('::coin::Coin<');
  }

  /// Encode string to BCS format
  static List<int> encodeString(String value) {
    final bcs = Bcs.string();
    return bcs.serialize(value).toBytes();
  }

  /// Decode string from BCS bytes
  ///
  /// Format: ULEB128 length + UTF-8 bytes
  static String decodeString(List<int> bytes) {
    if (bytes.isEmpty) {
      throw ArgumentError('Cannot decode string from empty bytes');
    }

    // Decode ULEB128 length
    int length = 0;
    int shift = 0;
    int index = 0;

    while (index < bytes.length) {
      final byte = bytes[index];
      length |= (byte & 0x7F) << shift;
      index++;

      if ((byte & 0x80) == 0) {
        break;
      }
      shift += 7;
    }

    // Decode UTF-8 string
    final stringBytes = bytes.sublist(index, index + length);
    return String.fromCharCodes(stringBytes);
  }

  /// Encode boolean to BCS format
  /// true = [1], false = [0]
  static List<int> encodeBool(bool value) {
    return value ? [1] : [0];
  }

  /// Decode boolean from BCS bytes
  static bool decodeBool(List<int> bytes) {
    if (bytes.isEmpty) {
      throw ArgumentError('Cannot decode boolean from empty bytes');
    }
    return bytes[0] == 1;
  }

  /// Extract coin type from object type string
  static String? extractCoinTypeFromObjectType(String objectType) {
    final normalized = objectType.replaceAll(' ', '');
    if (!isCoinObjectType(normalized)) {
      return null;
    }

    final start = normalized.indexOf('<');
    final end = normalized.lastIndexOf('>');

    if (start != -1 && end != -1 && end > start) {
      final inner = normalized.substring(start + 1, end);

      // Check if inner type contains another nested generic (Coin<TokenType>)
      if (inner.contains('<')) {
        // Format: Coin<TokenType> → extract TokenType
        final nestedStart = inner.indexOf('<');
        final nestedEnd = inner.lastIndexOf('>');
        if (nestedStart != -1 && nestedEnd != -1) {
          return inner.substring(nestedStart + 1, nestedEnd).trim();
        }
      }

      return inner.trim();
    }

    return null;
  }

  /// Validate address format
  static bool isValidAddress(String addr) {
    try {
      normalizeAddress(addr);
      return true;
    } catch (_) {
      return false;
    }
  }

  /// Validate object ID format
  static bool isValidObjectId(String objectId) {
    try {
      normalizeObjectId(objectId);
      return true;
    } catch (_) {
      return false;
    }
  }

  /// Normalize token type format (address::module::struct)
  static String normalizeTokenType(String tokenType) {
    final parts = tokenType.split('::');
    if (parts.length < 3) {
      throw ArgumentError(
        'Invalid token format. Expected: address::module::struct',
      );
    }

    // Normalize package address
    var packageAddr = parts[0];

    // Handle short address format (0x2, 0x1, etc.)
    // Short addresses are valid and should be kept as-is
    if (packageAddr.startsWith('0x')) {
      final hexPart = packageAddr.substring(2);
      // If it's a short address (< 64 chars), keep it as-is
      if (hexPart.length < 64) {
        // Already normalized short address
        return '$packageAddr::${parts[1]}::${parts[2]}';
      }
      // Full address - validate and normalize
      packageAddr = normalizeAddress(packageAddr);
    } else {
      // No 0x prefix - add it
      packageAddr = '0x$packageAddr';
      final hexPart = packageAddr.substring(2);
      if (hexPart.length < 64) {
        // Short address
        return '$packageAddr::${parts[1]}::${parts[2]}';
      }
      packageAddr = normalizeAddress(packageAddr);
    }

    return '$packageAddr::${parts[1]}::${parts[2]}';
  }

  /// Canonical token type for equality checks.
  /// Keeps the type path intact but expands the package address to 32 bytes.
  static String canonicalTokenType(String tokenType) {
    final normalized = normalizeTokenType(tokenType);
    final parts = normalized.split('::');
    final packageAddr = normalizeAddress(parts[0]);
    return '$packageAddr::${parts.sublist(1).join('::')}';
  }

  /// Compare token types while accepting both short and full package addresses.
  static bool tokenTypesEqual(String left, String right) {
    return canonicalTokenType(left) == canonicalTokenType(right);
  }
}

/// Transaction argument builder (chainable)
class TransactionArgs {
  final List<List<int>> _args = [];

  TransactionArgs addAddress(String address) {
    final normalized = BcsUtils.normalizeAnyAddress(address);
    final hexPart = normalized.substring(2);
    // Pad to 64 characters for BCS encoding
    final padded = hexPart.padLeft(64, '0');
    _args.add(BcsUtils.hexToBytes('0x$padded'));
    return this;
  }

  TransactionArgs addObjectId(String objectId) {
    final normalized = BcsUtils.normalizeAnyAddress(objectId);
    final hexPart = normalized.substring(2);
    // Pad to 64 characters for BCS encoding
    final padded = hexPart.padLeft(64, '0');
    _args.add(BcsUtils.hexToBytes('0x$padded'));
    return this;
  }

  TransactionArgs addAmount(int amount) {
    _args.add(BcsUtils.encodeU64(amount));
    return this;
  }

  TransactionArgs addString(String value) {
    _args.add(BcsUtils.encodeString(value));
    return this;
  }

  TransactionArgs addBool(bool value) {
    _args.add(BcsUtils.encodeBool(value));
    return this;
  }

  TransactionArgs addBytes(List<int> bytes) {
    _args.add(bytes);
    return this;
  }

  TransactionArgs addHex(String hexStr) {
    _args.add(BcsUtils.hexToBytes(hexStr));
    return this;
  }

  List<List<int>> build() => List.from(_args);

  void clear() => _args.clear();

  int get length => _args.length;

  bool get isEmpty => _args.isEmpty;

  bool get isNotEmpty => _args.isNotEmpty;
}
