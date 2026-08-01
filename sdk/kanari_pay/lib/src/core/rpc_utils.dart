// core/rpc_utils.dart
// RPC utilities for Kanari SDK.

import 'dart:convert';
import 'package:http/http.dart' as http;
import '../models/rpc_response.dart';

class RpcUtils {
  const RpcUtils._();

  /// Make RPC request with error handling
  static Future<RpcResponse<T>> request<T>(
    http.Client client,
    String url,
    String method,
    dynamic params,
    T Function(Object? json) fromJsonT,
  ) async {
    final body = {
      'jsonrpc': '2.0',
      'method': method,
      'params': params,
      'id': DateTime.now().millisecondsSinceEpoch,
    };

    final response = await client.post(
      Uri.parse(url),
      headers: {'Content-Type': 'application/json'},
      body: jsonEncode(body),
    );

    if (response.statusCode != 200) {
      throw Exception(
        'Failed to connect to Kanari RPC: ${response.statusCode}',
      );
    }

    final jsonResponse = jsonDecode(response.body) as Map<String, dynamic>;
    return RpcResponse<T>.fromJson(jsonResponse, fromJsonT);
  }

  /// Execute view function via RPC
  static Future<List<dynamic>> executeViewFunction(
    http.Client client,
    String url,
    String packageAddress,
    String module,
    String functionName,
    List<String> typeArgs,
    List<List<int>> arguments, {
    String sender = '',
    List<Map<String, dynamic>> objectInputs = const [],
  }) async {
    // Convert args to hex strings for RPC
    final argsHex = arguments
        .map(
          (bytes) =>
              '0x${bytes.map((b) => b.toRadixString(16).padLeft(2, '0')).join()}',
        )
        .toList();

    // Build request data object
    final requestData = {
      'sender': sender,
      'package': packageAddress,
      'module': module,
      'function': functionName,
      'type_args': typeArgs,
      'args': argsHex,
      if (objectInputs.isNotEmpty) 'object_inputs': objectInputs,
    };

    // params must be an ARRAY containing the request object
    final body = {
      'jsonrpc': '2.0',
      'method': 'kanari_viewFunction',
      'params': [requestData],
      'id': DateTime.now().millisecondsSinceEpoch,
    };

    final response = await client.post(
      Uri.parse(url),
      headers: {'Content-Type': 'application/json'},
      body: jsonEncode(body),
    );

    if (response.statusCode != 200) {
      throw Exception('View function failed: ${response.statusCode}');
    }

    final jsonResponse = jsonDecode(response.body) as Map<String, dynamic>;

    if (jsonResponse.containsKey('error')) {
      throw Exception('View function error: ${jsonResponse['error']}');
    }

    final result = jsonResponse['result'];

    if (result is List) {
      return result;
    }

    return [result];
  }
}
