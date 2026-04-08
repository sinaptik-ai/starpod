import { useState, useCallback, useEffect } from 'react'
import { StarpodIcon } from '../ui/Logo'
import StepAgent from './StepAgent'
import StepConnections from './StepConnections'
import StepRole from './StepRole'
import StepDone from './StepDone'

const STEPS = ['Agent', 'Connections', 'Role']

export default function OnboardingWizard({ initialData, onComplete }) {
  const [phase, setPhase] = useState('intro') // intro | steps | done
  const [step, setStep] = useState(0)
  const [transitioning, setTransitioning] = useState(false)
  const [data, setData] = useState({
    agentName: initialData?.agent_name || 'Nova',
    provider: initialData?.provider || 'anthropic',
    model: '',
    browserEnabled: false,
    connectors: [],
    generatedSkills: [],
  })

  const updateData = useCallback((updates) => {
    setData(prev => ({ ...prev, ...updates }))
  }, [])

  const animateTransition = useCallback((newStep) => {
    setTransitioning(true)
    setTimeout(() => {
      if (newStep >= STEPS.length) {
        setPhase('done')
      } else {
        setStep(newStep)
      }
      setTransitioning(false)
    }, 150)
  }, [])

  const next = useCallback(
    () => animateTransition(step + 1),
    [step, animateTransition],
  )
  const back = useCallback(
    () => animateTransition(Math.max(0, step - 1)),
    [step, animateTransition],
  )

  if (phase === 'intro') {
    return <IntroScreen onStart={() => setPhase('steps')} />
  }

  if (phase === 'done') {
    return <StepDone agentName={data.agentName} onComplete={onComplete} />
  }

  let stepComponent = null
  if (step === 0) {
    stepComponent = <StepAgent data={data} updateData={updateData} onNext={next} />
  } else if (step === 1) {
    stepComponent = (
      <StepConnections
        data={data}
        updateData={updateData}
        onNext={next}
        onBack={back}
      />
    )
  } else if (step === 2) {
    stepComponent = (
      <StepRole data={data} updateData={updateData} onNext={next} onBack={back} />
    )
  }

  return (
    <div className="ob-shell">
      <div className="ob-header">
        <div className="ob-progress">
          {STEPS.map((label, i) => (
            <div key={label} className="ob-progress-step">
              <div
                className={`ob-progress-bar ${i <= step ? 'ob-progress-bar--filled' : ''}`}
              />
              <span
                className={`ob-progress-label ${i <= step ? 'ob-progress-label--active' : ''}`}
              >
                {label}
              </span>
            </div>
          ))}
        </div>
      </div>

      <div className="ob-content-area">
        <div className={`ob-step-container ${transitioning ? 'ob-fading' : ''}`}>
          {stepComponent}
        </div>
      </div>
    </div>
  )
}

function IntroScreen({ onStart }) {
  const [visible, setVisible] = useState(false)

  useEffect(() => {
    const t = setTimeout(() => setVisible(true), 50)
    return () => clearTimeout(t)
  }, [])

  return (
    <div className={`ob-intro ${visible ? 'ob-intro--visible' : ''}`}>
      <div className="ob-intro-content">
        <StarpodIcon className="w-12 h-12 text-primary" />
        <h1 className="ob-intro-brand">Starpod</h1>
        <p className="ob-intro-sub">Set up your agent</p>
        <button className="ob-intro-btn" onClick={onStart} autoFocus type="button">
          Begin
        </button>
      </div>
    </div>
  )
}
