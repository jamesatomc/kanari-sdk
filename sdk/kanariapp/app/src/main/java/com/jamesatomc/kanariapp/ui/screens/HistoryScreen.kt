package com.jamesatomc.kanariapp.ui.screens

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.material.icons.filled.History
import androidx.compose.material.icons.filled.NorthEast
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.SouthWest
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.jamesatomc.kanariapp.network.models.TransactionDetails
import com.jamesatomc.kanariapp.ui.components.DetailRowShared
import com.jamesatomc.kanariapp.ui.components.LoadingEmptyState
import com.jamesatomc.kanariapp.ui.components.copyToClipboard
import com.jamesatomc.kanariapp.ui.components.formatAmount
import com.jamesatomc.kanariapp.wallet.WalletViewModel

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun HistoryScreen(viewModel: WalletViewModel) {
    val transactions by viewModel.transactions.collectAsStateWithLifecycle()
    val isLoading by viewModel.isLoading.collectAsStateWithLifecycle()
    val activeWallet by viewModel.activeWallet.collectAsStateWithLifecycle()
    var selectedTx by remember { mutableStateOf<TransactionDetails?>(null) }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("History", style = MaterialTheme.typography.titleLarge) },
                actions = {
                    IconButton(onClick = { viewModel.refreshBalance() }) {
                        Icon(Icons.Default.Refresh, contentDescription = "Refresh")
                    }
                }
            )
        }
    ) { padding ->
        LoadingEmptyState(
            isLoading = isLoading,
            items = transactions,
            emptyIcon = Icons.Default.History,
            emptyText = "No Transactions Yet",
            modifier = Modifier.padding(padding)
        ) {
            LazyColumn(modifier = Modifier.fillMaxSize()) {
                items(transactions, key = { it.hash }) { tx ->
                    HistoryItem(tx, tx.isIncomingTo(activeWallet?.address), onClick = { selectedTx = tx })
                }
            }
        }
    }

    selectedTx?.let { tx ->
        val isIncoming = tx.isIncomingTo(activeWallet?.address)
        ModalBottomSheet(onDismissRequest = { selectedTx = null }) {
            TransactionDetailSheet(tx = tx, isIncoming = isIncoming, onDismiss = { selectedTx = null })
        }
    }
}

private fun TransactionDetails.isIncomingTo(walletAddress: String?): Boolean {
    val myAdder = walletAddress?.lowercase() ?: return false
    return !(myAdder == sender.lowercase() || myAdder == senderAddress?.lowercase())
}

@Composable
fun HistoryItem(tx: TransactionDetails, isIncoming: Boolean, onClick: () -> Unit) {
    ListItem(
        headlineContent = { Text(tx.txType) },
        supportingContent = {
            Text(
                if (isIncoming) "From: ${tx.sender.take(8)}..." else "Sent • ${tx.status}",
                maxLines = 1
            )
        },
        leadingContent = {
            Icon(
                imageVector = if (isIncoming) Icons.Default.SouthWest else Icons.Default.NorthEast,
                contentDescription = null,
                tint = if (isIncoming) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.error
            )
        },
        trailingContent = {
            val gasAmount = tx.effects?.gasUsed ?: tx.gasUsed ?: 0L
            Text(
                text = "${if (isIncoming) "+" else "-"}${formatAmount(gasAmount, 9, 2)}",
                color = if (isIncoming) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurface
            )
        },
        modifier = Modifier.clickable(onClick = onClick)
    )
}

@Composable
fun TransactionDetailSheet(tx: TransactionDetails, isIncoming: Boolean, onDismiss: () -> Unit) {
    val context = LocalContext.current
    Column(
        modifier = Modifier.fillMaxWidth().padding(16.dp).verticalScroll(rememberScrollState()).navigationBarsPadding(),
        verticalArrangement = Arrangement.spacedBy(12.dp)
    ) {
        Text("Transaction Details", style = MaterialTheme.typography.titleLarge, fontWeight = FontWeight.Bold)
        HorizontalDivider()
        DetailRowShared(label = "Type", value = tx.txType)
        DetailRowShared(
            label = "Status", value = tx.status, valueColor = when (tx.status.lowercase()) {
                "committed", "success" -> MaterialTheme.colorScheme.primary; "failed" -> MaterialTheme.colorScheme.error; else -> MaterialTheme.colorScheme.onSurface
            }
        )
        DetailRowShared(label = "Direction", value = if (isIncoming) "Incoming" else "Outgoing")
        CopyableDetailRow(label = "Hash", value = tx.hash)
        CopyableDetailRow(label = "Sender", value = tx.sender)
        tx.senderAddress?.let { CopyableDetailRow(label = "Sender Address", value = it) }
        tx.module?.let { DetailRowShared(label = "Module", value = it) }
        tx.function?.let { DetailRowShared(label = "Function", value = it) }
        DetailRowShared(label = "Nonce", value = tx.nonce.toString())
        DetailRowShared(label = "Gas Limit", value = tx.gasLimit.toString())
        DetailRowShared(label = "Gas Price", value = tx.gasPrice.toString())
        tx.gasUsed?.let { DetailRowShared(label = "Gas Used", value = it.toString()) }
        tx.blockHeight?.let { DetailRowShared(label = "Block Height", value = it.toString()) }
        tx.checkpointHeight?.let { DetailRowShared(label = "Checkpoint", value = it.toString()) }
        tx.effects?.let { eff ->
            DetailRowShared(label = "Effects Status", value = eff.status)
            DetailRowShared(label = "Effects Gas", value = eff.gasUsed.toString())
            if (eff.objectChanges.isNotEmpty()) {
                Text(
                    "Object Changes (${eff.objectChanges.size})",
                    style = MaterialTheme.typography.titleSmall,
                    fontWeight = FontWeight.Bold,
                    modifier = Modifier.padding(top = 8.dp)
                )
                eff.objectChanges.take(10).forEach { ch ->
                    Surface(
                        shape = MaterialTheme.shapes.small,
                        color = MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.5f),
                        modifier = Modifier.fillMaxWidth()
                    ) {
                        Column(Modifier.padding(8.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                            Text(ch.changeType, style = MaterialTheme.typography.labelMedium)
                            Text(
                                ch.objectRef.objectId,
                                style = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace),
                                maxLines = 1
                            )
                            ch.type?.let {
                                Text(
                                    it,
                                    style = MaterialTheme.typography.bodySmall,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant
                                )
                            }
                        }
                    }
                }
            }
        }
        tx.moduleFunctions?.takeIf { it.isNotEmpty() }?.let { fns ->
            Text(
                "Functions",
                style = MaterialTheme.typography.titleSmall,
                fontWeight = FontWeight.Bold,
                modifier = Modifier.padding(top = 8.dp)
            )
            fns.take(10).forEach { Text("• $it", style = MaterialTheme.typography.bodySmall) }
        }
        Spacer(Modifier.height(8.dp))
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            OutlinedButton(
                onClick = { copyToClipboard(context, tx.hash, toast = "Hash copied"); },
                modifier = Modifier.weight(1f)
            ) {
                Icon(Icons.Default.ContentCopy, contentDescription = null, modifier = Modifier.size(18.dp))
                Spacer(Modifier.width(8.dp)); Text("Copy Hash")
            }
            Button(onClick = onDismiss, modifier = Modifier.weight(1f)) { Text("Close") }
        }
        Spacer(Modifier.height(12.dp))
    }
}

@Composable
private fun CopyableDetailRow(label: String, value: String) {
    val context = LocalContext.current
    Column(Modifier.fillMaxWidth()) {
        Text(label, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Row(
            Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.SpaceBetween
        ) {
            Text(
                value,
                style = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace),
                modifier = Modifier.weight(1f).padding(top = 2.dp, end = 8.dp),
                maxLines = 3
            )
            IconButton(onClick = { copyToClipboard(context, value) }, modifier = Modifier.size(28.dp)) {
                Icon(
                    Icons.Default.ContentCopy,
                    contentDescription = "Copy",
                    modifier = Modifier.size(16.dp),
                    tint = MaterialTheme.colorScheme.primary
                )
            }
        }
    }
}