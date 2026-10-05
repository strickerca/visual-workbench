package com.visualworkbench.android.remote

import com.visualworkbench.shared.RemoteTargetBinding
import org.junit.Assert.*
import org.junit.Test

class RemoteInputGrantFenceTest {
    private val binding=RemoteTargetBinding(1uL,"capture",1uL,"target",1u,"input")
    @Test fun delayed_same_grant_renderer_cannot_readmit_after_local_cancellation(){
        val gate=RemoteInputGrantFence();assertTrue(gate.permits(binding))
        gate.invalidate(binding);assertFalse(gate.permits(binding))
        gate.invalidate(null);assertFalse(gate.permits(binding))
    }
    @Test fun only_fresh_explicit_input_binding_can_reopen_local_fence(){
        val gate=RemoteInputGrantFence();gate.invalidate(binding)
        assertFalse(gate.permits(binding.copy()))
        assertTrue(gate.permits(binding.copy(inputSessionId="fresh-explicit-grant")))
    }
}
