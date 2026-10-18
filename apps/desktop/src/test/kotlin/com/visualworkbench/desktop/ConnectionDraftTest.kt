package com.visualworkbench.desktop

import com.visualworkbench.shared.LocalSessionAddress
import com.visualworkbench.shared.PairedDevice
import com.visualworkbench.shared.ProjectSessionRole
import com.visualworkbench.shared.SessionCarrier
import org.junit.Assert.*
import org.junit.Test
import java.nio.file.Path

class ConnectionDraftTest {
    private val usb = LocalSessionAddress("192.0.2.10", 3u, "Phone connection")
    private val phone = PairedDevice("phone", "22".repeat(32), 1uL, null)
    private val host = ProjectSessionRole("project", "pc", "pc", true, 0u, 0u)

    @Test fun pickerReturnIsLimitedToConnectionOriginAndNeverReplacesAnotherModal() {
        assertEquals(Command.Pairing, dialogAfterProjectPicker(true, null, false))
        assertNull(dialogAfterProjectPicker(false, null, false))
        assertNull(dialogAfterProjectPicker(true, null, true))
        assertEquals(Command.Settings, dialogAfterProjectPicker(true, Command.Settings, false))
        assertEquals(Command.Open, dialogAfterProjectPicker(true, Command.Open, false))
    }

    @Test fun adapterRefreshRetiresCopiedUsbWithoutSubstitutingOrDroppingManualRoutes() {
        val manual = EndpointFields(wifi = "192.0.2.80:47000", adb = "127.0.0.1:47191")
        val remote = EndpointFields(tether = "192.0.2.90:48000")
        val selected = ConnectionDraft(local = manual, remote = remote, method = PairingMethod.Code,
            expectedProject = "wanted-project").selectPeer(phone).selectConnection(usb)
            .useSelectedUsb(listOf(usb), false)
        val renamed = usb.copy(interfaceName = "Renamed adapter")
        val refreshed = selected.reconcileConnections(listOf(renamed))
        assertEquals("Renamed adapter", refreshed.selectedAddress?.interfaceName)
        assertEquals(pairingEndpoint(usb), refreshed.local.tether)
        assertTrue(refreshed.issues.isEmpty())

        val replacement = usb.copy(address = "192.0.2.11")
        val retired = refreshed.reconcileConnections(listOf(replacement))
        assertNull(retired.selectedAddress)
        assertEquals("", retired.bind)
        assertEquals("", retired.local.tether)
        assertNull(retired.copiedUsbSource)
        assertEquals(manual, retired.local)
        assertEquals(remote, retired.remote)
        assertEquals(phone, retired.selectedPeer(listOf(phone)))
        assertEquals(PairingMethod.Code, retired.method)
        assertEquals("wanted-project", retired.expectedProject)
        assertTrue(retired.issues.contains(ConnectionDraftIssue.CopiedUsbRetired))
        assertTrue(retired.issues.contains(ConnectionDraftIssue.ConnectionUnavailable))
    }

    @Test fun explicitAdapterReplacementRequiresAnotherUsbChoiceAndManualTetherSurvives() {
        val replacement = usb.copy(interfaceIndex = 4u)
        val selected = ConnectionDraft().selectPeer(phone).selectConnection(usb)
            .useSelectedUsb(listOf(usb), false)
        val changed = selected.selectConnection(replacement)
        assertEquals(replacement, changed.selectedAddress)
        assertEquals("", changed.local.tether)
        assertEquals(ProjectConnectionBlocker.ProjectEndpoint, blocker(changed, listOf(replacement)))
        val explicit = changed.useSelectedUsb(listOf(replacement), false)
        assertNull(blocker(explicit, listOf(replacement)))

        val manual = selected.manualLocalTether("192.0.2.77:48000")
            .copy(bind = "192.0.2.88:44000")
        val missing = manual.reconcileConnections(listOf(replacement))
        assertNull(missing.selectedAddress)
        assertNull(missing.copiedUsbSource)
        assertEquals("192.0.2.77:48000", missing.local.tether)
        assertEquals("192.0.2.88:44000", missing.bind)
        assertEquals(ProjectConnectionBlocker.SelectConnection, blocker(missing, listOf(replacement)))
        assertFalse(missing.issues.contains(ConnectionDraftIssue.CopiedUsbRetired))
    }

    @Test fun peerSelectionRequiresTheFullCurrentCertificateAndActiveTrust() {
        val selected = ConnectionDraft(remote = EndpointFields(wifi = "192.0.2.70:47000")).selectPeer(phone)
        val renewed = phone.copy(fingerprint = "33".repeat(32))
        val revoked = phone.copy(revokedAtMs = 2uL)
        for (current in listOf(emptyList(), listOf(renewed), listOf(revoked))) {
            assertNull(selected.selectedPeer(current))
            val retired = selected.reconcilePeers(current)
            assertNull(retired.peer)
            assertEquals(selected.remote, retired.remote)
            assertTrue(retired.issues.contains(ConnectionDraftIssue.PeerUnavailable))
        }
        assertEquals(phone, selected.reconcilePeers(listOf(phone)).selectedPeer(listOf(phone)))
        val deliberate = selected.selectPeer(renewed)
        assertEquals(renewed, deliberate.selectedPeer(listOf(renewed)))
        assertNull(deliberate.selectedPeer(listOf(phone)))
        assertNull(selected.copy(peer = ConnectionPeer(phone.deviceId, phone.fingerprint.take(8))).selectedPeer(listOf(phone)))
    }

    @Test fun unknownRoleAndReplicaPeerMismatchNeverEnableProjectConnection() {
        val draft = ConnectionDraft(local = EndpointFields(wifi = "192.0.2.60:47000"),
            remote = EndpointFields(wifi = "192.0.2.61:47000")).selectPeer(phone)
        assertEquals(ProjectConnectionBlocker.ProjectRole, blocker(draft, role = null))
        val replica = host.copy(localDeviceId = "replica", hostDeviceId = "original-host", isHost = false)
        assertEquals(ProjectConnectionBlocker.OriginalHost, blocker(draft, role = replica))
        val original = phone.copy(deviceId = "original-host")
        val explicit = draft.selectPeer(original)
        assertNull(explicit.projectBlocker(true, false, false, true, replica, listOf(phone, original), emptyList(), false))
        assertEquals(ProjectConnectionBlocker.OpenProject, draft.projectBlocker(true, false, false, false,
            host, listOf(phone), emptyList(), false))
    }

    @Test fun onlyExplicitUsbChoiceCompletesHostReadinessAndKeepsCarrierPriority() {
        val selected = ConnectionDraft().selectPeer(phone).selectConnection(usb)
        assertTrue(selected.local.endpoints().isEmpty())
        assertEquals(ProjectConnectionBlocker.ProjectEndpoint, blocker(selected))
        assertEquals(ProjectConnectionBlocker.RefreshingConnections,
            selected.projectBlocker(true, false, false, true, host, listOf(phone), listOf(usb), true, true))
        assertEquals(selected, selected.useSelectedUsb(listOf(usb), true))
        val manual = selected.copy(local = EndpointFields(wifi = "192.0.2.50:47000", adb = "127.0.0.1:47191"))
        val configured = manual.useSelectedUsb(listOf(usb), false)
        assertNull(blocker(configured))
        assertEquals(SessionCarrier.entries, configured.local.endpoints().map { it.carrier })
        assertEquals(pairingEndpoint(usb), configured.local.tether)
        assertEquals(manual.local.wifi, configured.local.wifi)
        assertEquals(manual.local.adb, configured.local.adb)
        val customPort = selected.copy(bind = "192.0.2.10:44000")
        assertEquals("192.0.2.10:44000", customPort.useSelectedUsb(listOf(usb), false).local.tether)
        val unrelated = selected.copy(bind = "192.0.2.99:44000")
        assertEquals(unrelated, unrelated.useSelectedUsb(listOf(usb), false))
        assertEquals(ProjectConnectionBlocker.PairingEndpointMismatch,
            unrelated.projectBlocker(true, false, false, true, host, listOf(phone), listOf(usb), false, true))
        assertEquals(ProjectConnectionBlocker.ProjectBusy,
            configured.projectBlocker(true, false, true, true, host, listOf(phone), listOf(usb), false))
        assertEquals(ProjectConnectionBlocker.PairingActive,
            configured.projectBlocker(true, true, false, true, host, listOf(phone), listOf(usb), false))
        assertEquals(ProjectConnectionBlocker.PairingStorage,
            configured.projectBlocker(false, false, false, true, host, listOf(phone), listOf(usb), false))
    }

    @Test fun projectPickerAcceptsOnlyTheDatabaseFileWithoutFilesystemMutation() {
        val directory = Path.of("projects", "saved.vwb")
        assertEquals(directory, projectDirectoryFromPicker(directory.resolve("project.sqlite")))
        assertEquals(directory, projectDirectoryFromPicker(directory.resolve("PROJECT.SQLITE")))
        assertNull(projectDirectoryFromPicker(directory.resolve("original.png")))
        assertNull(projectDirectoryFromPicker(directory.resolve("project.sqlite-wal")))
        assertNull(projectDirectoryFromPicker(directory))
        assertNull(projectDirectoryFromPicker(Path.of("project.sqlite")))
    }

    @Test fun explicitAdvancedReceiveStillRequiresTrustAndRouteSafetyAndDraftsRedact() {
        val draft = ConnectionDraft(remote = EndpointFields(tether = "192.0.2.90:48000"),
            expectedProject = "private-project").selectPeer(phone).selectConnection(usb)
        assertNull(draft.receiveBlocker(true, false, false, listOf(phone), listOf(usb), false))
        assertEquals(ProjectConnectionBlocker.SelectConnection,
            draft.receiveBlocker(true, false, false, listOf(phone), emptyList(), false))
        assertEquals(ProjectConnectionBlocker.SelectPeer,
            draft.receiveBlocker(true, false, false, listOf(phone.copy(revokedAtMs = 2uL)), listOf(usb), false))
        assertEquals("ConnectionDraft([REDACTED])", draft.toString())
        assertEquals("ConnectionPeer([REDACTED])", draft.peer.toString())
        assertEquals("EndpointFields([REDACTED])", draft.remote.toString())
        assertFalse(draft.toString().contains(usb.address))
        assertFalse(draft.toString().contains(phone.fingerprint))
    }

    @Test fun savedPhonePresentationDoesNotStartPairingOrChooseItsPeerAndRoute() {
        val untouched = ConnectionDraft()
        assertEquals(ConnectionScreenPath.Loading, connectionScreenPath(false, listOf(phone), null, false))
        assertEquals(ConnectionScreenPath.PairedProject, connectionScreenPath(true, listOf(phone), null, false))
        assertEquals(ConnectionScreenPath.PairNewPhone, connectionScreenPath(true, emptyList(), null, false))
        assertEquals(ConnectionScreenPath.PairNewPhone,
            connectionScreenPath(true, listOf(phone.copy(revokedAtMs = 2uL)), null, false))
        assertEquals(ConnectionScreenPath.PairNewPhone, connectionScreenPath(true, listOf(phone), true, false))
        assertEquals(ConnectionScreenPath.PairedProject, connectionScreenPath(true, emptyList(), false, false))
        assertEquals(ConnectionScreenPath.PairNewPhone, connectionScreenPath(true, listOf(phone), false, true))
        assertNull(untouched.peer)
        assertNull(untouched.selectedAddress)
        assertTrue(untouched.local.endpoints().isEmpty())
        assertTrue(untouched.remote.endpoints().isEmpty())
    }

    @Test fun pairingReadinessChecksOnlyTheChosenMethodsExplicitInputs() {
        val qr = ConnectionDraft()
        assertEquals(PairingActionBlocker.Storage, qr.pairingBlocker(false, false, false, ""))
        assertEquals(PairingActionBlocker.Active, qr.pairingBlocker(true, true, false, ""))
        assertEquals(PairingActionBlocker.Refreshing, qr.pairingBlocker(true, false, true, ""))
        assertEquals(PairingActionBlocker.LocalConnection, qr.pairingBlocker(true, false, false, ""))
        val explicitlySelected = qr.selectConnection(usb)
        assertNull(explicitlySelected.pairingBlocker(true, false, false, ""))
        assertNull(explicitlySelected.copy(method = PairingMethod.Code).pairingBlocker(true, false, false, ""))
        val join = qr.copy(method = PairingMethod.Join)
        assertEquals(PairingActionBlocker.OtherEndpoint, join.pairingBlocker(true, false, true, "12345678"))
        val other = join.copy(peerEndpoint = "192.0.2.60:44242")
        assertEquals(PairingActionBlocker.Code, other.pairingBlocker(true, false, true, "1234567"))
        assertEquals(PairingActionBlocker.Code, other.pairingBlocker(true, false, false, "1234567x"))
        assertNull(other.pairingBlocker(true, false, true, "12345678"))
        assertNull(other.selectedAddress)
        assertNull(other.peer)
    }

    @Test fun computerAndAdapterDescriptionsNeverBecomeDeviceOrRouteAuthority() {
        assertEquals("MY-DESKTOP", computerLabel(" MY-DESKTOP "))
        assertEquals(computerLabel(null), computerLabel("unsafe\nname"))
        assertEquals(computerLabel(null), computerLabel(" "))
        val adapter = usb.copy(interfaceName = "Ethernet adapter")
        val sameDescription = adapter.copy(interfaceIndex = 4u, address = "192.0.2.11")
        val inventory = listOf(adapter, sameDescription)
        assertTrue(adapterLabel(adapter, inventory).contains("Ethernet adapter"))
        assertNotEquals(adapterLabel(adapter, inventory), adapterLabel(sameDescription, inventory))
        assertFalse(adapterLabel(adapter, inventory).contains(adapter.address))
        assertFalse(networkConnectionLabel(adapter, inventory).contains(adapter.address))
        val unnamed = usb.copy(interfaceName = " ")
        assertTrue(adapterLabel(unnamed, listOf(unnamed)).isNotBlank())
        assertNotEquals(addressOptionLabel(usb, listOf(usb)),
            addressOptionLabel(usb.copy(address = "2001:db8::10"), listOf(usb.copy(address = "2001:db8::10"))))
        assertTrue(adapter.sameConnection(adapter.copy(interfaceName = "A different description")))
        assertFalse(adapter.sameConnection(sameDescription))
        assertEquals(usb.address, adapter.address)
        assertEquals(usb.interfaceIndex, adapter.interfaceIndex)
    }

    private fun blocker(draft: ConnectionDraft, addresses: List<LocalSessionAddress> = listOf(usb),
        role: ProjectSessionRole? = host): ProjectConnectionBlocker? =
        draft.projectBlocker(true, false, false, true, role, listOf(phone), addresses, false)
}
