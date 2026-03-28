import { StarpodIcon } from '../ui/Logo'

export default function StepDone({ agentName, onComplete }) {
  return (
    <div className="ob-done">
      <StarpodIcon className="w-14 h-14 text-primary" />
      <h2 className="ob-done-name">{agentName} is ready</h2>
      <p className="ob-done-sub">
        Your agent is configured and waiting. Start a conversation to get going.
      </p>
      <button className="ob-btn-primary ob-done-btn" onClick={onComplete} autoFocus>
        Start chatting
      </button>
    </div>
  )
}
